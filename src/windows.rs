//! The application owns preferences/persistence, clipboard and the GPU upload
//! throttle/device-loss observer. Each viewport owns a Viewer: decoder workers,
//! CPU cache, GPU image residency and paint bindings, selection, transform,
//! playback and action errors. Paint callbacks own their image Arc and bind groups;
//! no shared callback-resource slot or image texture is overwritten by another window.
//! Immediate viewports keep mutations on the UI thread. Settings are app preferences
//! (not per-window playback position/pause). The one shared upload throttle retains
//! the existing bounded staging contract, with a unique token per viewport pass.
//! A closed root becomes a hidden event-loop host, not an image window; its
//! Viewer is dropped just like a child. eframe exits only when root is closed.
use std::{collections::HashMap, path::PathBuf, sync::Arc};

use eframe::egui::{self, ViewportId};

use crate::app::{Shared, Viewer};

#[derive(Default)]
pub(crate) struct Action {
    pub new_window: bool,
    pub close: bool,
    pub quit: bool,
}

struct Registry<T> {
    windows: HashMap<ViewportId, T>,
    // Creation order keeps rendering and existing-document ownership deterministic.
    order: Vec<ViewportId>,
    focused: Option<ViewportId>,
    next: u64,
}

impl<T> Registry<T> {
    fn new(root: T) -> Self {
        Self {
            windows: HashMap::from([(ViewportId::ROOT, root)]),
            order: vec![ViewportId::ROOT],
            focused: Some(ViewportId::ROOT),
            next: 0,
        }
    }

    fn create(&mut self, window: T) -> ViewportId {
        self.next = self
            .next
            .checked_add(1)
            .expect("window ID counter exhausted");
        let id = ViewportId::from_hash_of(("image-window", self.next));
        self.windows.insert(id, window);
        self.order.push(id);
        id
    }

    fn focus(&mut self, id: ViewportId) {
        if self.windows.contains_key(&id) {
            self.focused = Some(id);
        }
    }

    fn close(&mut self, id: ViewportId) -> bool {
        self.windows.remove(&id);
        self.order.retain(|existing| *existing != id);
        if self.focused == Some(id) {
            self.focused = None;
        }
        self.windows.is_empty()
    }

    #[cfg(any(target_os = "macos", test))]
    fn target(&self) -> Option<ViewportId> {
        self.focused
            .filter(|id| self.windows.contains_key(id))
            .or_else(|| (self.order.len() == 1).then(|| self.order[0]))
    }

    #[cfg(any(target_os = "macos", test))]
    fn open_target(&self, owns: impl Fn(&T) -> bool) -> Option<ViewportId> {
        // Prefer the focused owner when the same file exists in several windows.
        self.target()
            .filter(|id| owns(&self.windows[id]))
            .or_else(|| {
                self.order
                    .iter()
                    .rev()
                    .copied()
                    .find(|id| owns(&self.windows[id]))
            })
            .or_else(|| self.target())
    }
}

pub(crate) struct Application {
    registry: Registry<Viewer>,
    shared: Shared,
    render_state: Option<eframe::egui_wgpu::RenderState>,
    uploads: Arc<std::sync::Mutex<crate::render::Uploads>>,
    staging_frame: u64,
    quit: bool,
    #[cfg(target_os = "macos")]
    pending_open: Option<PathBuf>,
}

pub(crate) fn configure_context(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());
    // egui's default Cmd-Q sends a viewport Close after UI, bypassing our
    // guarded app-wide quit path (and closing only a child). Own it instead.
    ctx.options_mut(|options| options.quit_shortcuts.clear());
}

impl Application {
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> Self {
        configure_context(&cc.egui_ctx);
        let (settings, store) = crate::settings::Store::load();
        #[cfg(target_os = "macos")]
        let initial = {
            crate::open_documents::register(
                objc2::MainThreadMarker::new().expect("viewer starts on main thread"),
            );
            crate::open_documents::attach_context(&cc.egui_ctx);
            let document = crate::open_documents::drain(initial.as_deref());
            if initial.is_none()
                && let Some(path) = document.as_deref()
            {
                crate::open_documents::trace_open(path);
            }
            initial.or(document)
        };
        let render_state = cc.wgpu_render_state.clone();
        let uploads = crate::render::Uploads::new(render_state.as_ref());
        let root = Viewer::new(
            cc.egui_ctx.clone(),
            render_state.clone(),
            Arc::clone(&uploads),
            settings,
            initial,
        );
        Self {
            registry: Registry::new(root),
            shared: Shared {
                settings,
                store,
                clipboard: None,
            },
            render_state,
            uploads,
            staging_frame: 0,
            quit: false,
            #[cfg(target_os = "macos")]
            pending_open: None,
        }
    }

    fn render(&mut self, id: ViewportId, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|input| input.focused) {
            self.registry.focus(id);
        }
        let Some(viewer) = self.registry.windows.get_mut(&id) else {
            return;
        };
        if id != ViewportId::ROOT {
            viewer.sync_settings(self.shared.settings);
            viewer.logic(&ctx);
        }
        // Shared upload budget must never confuse two independent viewport passes.
        self.staging_frame = self.staging_frame.wrapping_add(1);
        let action = viewer.ui(ui, frame, &mut self.shared, self.staging_frame);
        if action.quit {
            self.quit = true;
            return;
        }
        if action.new_window {
            let viewer = Viewer::new(
                ctx.clone(),
                self.render_state.clone(),
                Arc::clone(&self.uploads),
                self.shared.settings,
                None,
            );
            self.registry.create(viewer);
            ctx.request_repaint_of(ViewportId::ROOT);
        }
        if action.close {
            self.close(id, &ctx);
        }
    }

    fn close(&mut self, id: ViewportId, ctx: &egui::Context) {
        self.quit = self.registry.close(id);
        if id == ViewportId::ROOT && !self.quit {
            ctx.send_viewport_cmd_to(id, egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Visible(false));
        }
        ctx.request_repaint_of(ViewportId::ROOT);
    }
}

impl eframe::App for Application {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Raw viewport focus is current even before the next child UI pass.
        let focused = ctx.input(|input| {
            input
                .raw
                .viewports
                .iter()
                .find_map(|(id, info)| (info.focused == Some(true)).then_some(*id))
        });
        if let Some(id) = focused {
            self.registry.focus(id);
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(path) = crate::open_documents::drain(None) {
                self.pending_open = Some(path);
            }
            if let Some(path) = self.pending_open.as_ref()
                && let Some(id) = self
                    .registry
                    .open_target(|viewer| viewer.selected.as_ref() == Some(path))
            {
                let viewer = self
                    .registry
                    .windows
                    .get_mut(&id)
                    .expect("live open target");
                let path = self
                    .pending_open
                    .take()
                    .expect("pending open checked above");
                if viewer.selected.as_ref() != Some(&path) {
                    crate::open_documents::trace_open(&path);
                    viewer.open(path);
                }
            }
        }
        // eframe also calls logic without UI while the root is minimized/occluded.
        if ctx.input(|input| input.viewport().close_requested())
            && self.registry.windows.contains_key(&ViewportId::ROOT)
        {
            self.close(ViewportId::ROOT, ctx);
        }
        if let Some(viewer) = self.registry.windows.get_mut(&ViewportId::ROOT) {
            viewer.sync_settings(self.shared.settings);
            viewer.logic(ctx);
        } else if !self.quit {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, egui::ViewportCommand::CancelClose);
        }
        if self.quit {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, egui::ViewportCommand::Close);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.registry.windows.contains_key(&ViewportId::ROOT) {
            self.render(ViewportId::ROOT, ui, frame);
        } else if !self.quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        for id in self.registry.order.clone() {
            if id == ViewportId::ROOT || self.quit {
                continue;
            }
            ctx.show_viewport_immediate(
                id,
                egui::ViewportBuilder::default()
                    .with_title(self.registry.windows[&id].last_title.clone())
                    .with_inner_size([960.0, 640.0])
                    .with_min_inner_size([280.0, 180.0])
                    .with_icon(crate::window_icon()),
                |ui, _class| {
                    self.render(id, ui, frame);
                },
            );
        }
        if self.quit {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, egui::ViewportCommand::Close);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_focus_close_and_last_window_quit() {
        let mut registry = Registry::new("root");
        let first = registry.create("first");
        let second = registry.create("second");
        assert_ne!(first, second);
        assert_eq!(registry.order, [ViewportId::ROOT, first, second]);
        registry.focus(first);
        assert_eq!(registry.target(), Some(first));
        assert!(!registry.close(ViewportId::ROOT));
        assert_eq!(registry.windows[&first], "first");
        assert_eq!(registry.target(), Some(first));
        assert!(!registry.close(first));
        assert_eq!(registry.target(), Some(second));
        assert!(registry.close(second));
        assert_eq!(registry.target(), None);
    }

    #[test]
    fn open_target_uses_existing_owner_then_focus_then_sole_window() {
        let mut registry = Registry::new("root.png");
        assert_eq!(registry.target(), Some(ViewportId::ROOT));
        let first = registry.create("first.png");
        let second = registry.create("second.png");
        registry.focus(first);
        registry.focus(second);
        registry.focus(first);
        assert_eq!(
            registry.open_target(|path| *path == "second.png"),
            Some(second)
        );
        assert_eq!(registry.open_target(|path| *path == "new.png"), Some(first));
        registry.close(first);
        assert_eq!(registry.target(), None); // Two owners, no focus: don't guess.
        assert_eq!(
            registry.open_target(|path| *path == "second.png"),
            Some(second)
        );
        assert_eq!(registry.open_target(|path| *path == "new.png"), None);
        registry.focus(first); // Stale focus events cannot resurrect a closed window.
        assert_eq!(registry.target(), None);
        registry.close(second);
        assert_eq!(registry.target(), Some(ViewportId::ROOT));
    }

    #[test]
    fn duplicate_owners_prefer_focus_and_unknown_close_is_harmless() {
        let mut registry = Registry::new("same.png");
        let child = registry.create("same.png");
        assert_eq!(
            registry.open_target(|path| *path == "same.png"),
            Some(ViewportId::ROOT)
        );
        registry.focus(child);
        assert_eq!(
            registry.open_target(|path| *path == "same.png"),
            Some(child)
        );
        assert!(!registry.close(ViewportId::from_hash_of("unknown")));
        assert_eq!(registry.order, [ViewportId::ROOT, child]);
        assert!(!registry.close(child));
        assert!(!registry.close(child));
        assert!(registry.close(ViewportId::ROOT));
    }

    #[test]
    fn window_state_is_not_shared() {
        let mut registry = Registry::new(vec![1]);
        let child = registry.create(vec![2]);
        registry.windows.get_mut(&child).unwrap().push(3);
        assert_eq!(registry.windows[&ViewportId::ROOT], [1]);
        assert_eq!(registry.windows[&child], [2, 3]);
    }
}
