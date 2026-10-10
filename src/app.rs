use std::{path::PathBuf, sync::Arc};

use eframe::egui::{self, Color32, Key, Modifiers, Sense};

use crate::{
    cache::{Cache, Entry, Ready, neighbors},
    folder::{Direction, FileVersion, Folder, Sort},
    loader::{Completion, Loader, Request},
    render::Image,
    transform::Operation,
    view::View,
};

enum Status {
    Empty,
    Loading,
    Ready,
    Error(String),
}

// Admission owns the displayed resident independently of the pending selection.
// A ready resident is committed only after upload, before constructing any paint callback.
struct Display<T> {
    image: Option<T>,
    status: Status,
    generation: u64,
    path: Option<PathBuf>,
}

impl<T> Display<T> {
    fn new(status: Status) -> Self {
        Self {
            image: None,
            status,
            generation: 0,
            path: None,
        }
    }

    fn loading(&mut self) {
        self.status = Status::Loading;
    }

    fn fail(&mut self, error: String) {
        self.status = Status::Error(error);
    }

    fn clear(&mut self) {
        self.image = None;
        self.path = None;
        self.status = Status::Empty;
    }

    fn finish(
        &mut self,
        generation: u64,
        ready: Result<T, String>,
        path: Option<PathBuf>,
        view: &mut View,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        match ready {
            Ok(image) => {
                self.image = Some(image);
                if path.is_none() || self.path != path {
                    view.reset();
                }
                self.path = path;
                self.status = Status::Ready;
                true
            }
            Err(error) => {
                self.fail(error);
                false
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Command {
    Depth(usize),
    Open,
    Navigate(Direction),
    Sort(Sort),
    Fit,
    ActualSize,
    Zoom(f32),
    Transform(Operation),
    Reset,
    CopyImage,
    CopyFile,
    Paste,
    Reveal,
}

pub struct Viewer {
    prefetch: Option<Loader>,
    cache: Cache<Ready>,
    displayed_version: Option<FileVersion>,
    displayed_generation: u64,
    prefetch_epoch: u64,
    prefetch_queue: std::collections::VecDeque<PathBuf>,
    loader: Option<Loader>,
    folder: Option<Folder>,
    selected: Option<PathBuf>,
    texture_limit: u32,
    display: Display<Arc<Image>>,
    last_title: String,
    sort: Sort,
    view: View,
    render_state: Option<eframe::egui_wgpu::RenderState>,
    clipboard: Option<arboard::Clipboard>,
    action_error: Option<String>,
    reveal_child: Option<std::process::Child>,
}

impl Viewer {
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let (loader, display) = match Loader::new(cc.egui_ctx.clone()) {
            Ok(loader) => (Some(loader), Display::new(Status::Empty)),
            Err(error) => (
                None,
                Display::new(Status::Error(format!(
                    "Could not start image loader: {error}"
                ))),
            ),
        };
        let (prefetch, prefetch_error) = match Loader::new(cc.egui_ctx.clone()) {
            Ok(loader) => (Some(loader), None),
            Err(error) => (
                None,
                Some(format!(
                    "Preloading unavailable; could not start worker: {error}"
                )),
            ),
        };
        let mut viewer = Self {
            prefetch,
            cache: Cache::default(),
            displayed_version: None,
            displayed_generation: 0,
            prefetch_epoch: 0,
            prefetch_queue: Default::default(),
            loader,
            folder: None,
            selected: None,
            texture_limit: cc
                .wgpu_render_state
                .as_ref()
                .map_or(crate::decode::MAX_DIMENSION, |state| {
                    state.device.limits().max_texture_dimension_2d
                }),
            display,
            last_title: String::new(),
            sort: Sort::default(),
            view: View::default(),
            render_state: cc.wgpu_render_state.clone(),
            clipboard: None,
            action_error: prefetch_error,
            reveal_child: None,
        };
        if let Some(path) = initial {
            viewer.open(path);
        }
        viewer
    }

    fn open(&mut self, path: PathBuf) {
        if self
            .folder
            .as_ref()
            .is_none_or(|folder| path.parent() != Some(folder.parent()))
        {
            self.cache.clear();
        }
        self.folder = None;
        self.selected = Some(path.clone());
        self.display.loading();
        self.request(path, None);
    }

    fn request(&mut self, path: PathBuf, refresh: Option<(Folder, Option<Direction>)>) {
        self.stop_prefetch();
        let Some(generation) = self.display.generation.checked_add(1) else {
            self.display
                .fail("Selection counter exhausted; restart glance.".into());
            return;
        };
        self.display.generation = generation;
        if let Some(loader) = &self.loader {
            loader.request(Request {
                cached: self.cache.snapshot(),
                generation,
                path,
                scan_folder: refresh.is_none(),
                texture_limit: self.texture_limit,
                sort: self.sort,
                refresh,
                load_unchanged: matches!(self.display.status, Status::Loading),
            });
        } else {
            self.display
                .fail("Image loader unavailable; restart glance.".into());
        }
    }

    fn refresh(&mut self, direction: Option<Direction>) {
        if let Some(folder) = &mut self.folder {
            let snapshot = folder.clone();
            let path = self.selected.clone().unwrap_or_default();
            // Advance the provisional selection immediately so rapid keys build on
            // one another. The worker rescans the pre-command tombstone anchor.
            if let Some(direction) = direction
                && folder.navigate(direction)
            {
                self.selected = folder.selected_path().cloned();
                self.display.loading();
            }
            self.request(path, Some((snapshot, direction)));
        }
    }

    fn admit(&mut self, result: Completion) {
        if result.generation != self.display.generation {
            return;
        }
        if let Some(folder) = result.folder {
            match folder {
                Ok(folder) => {
                    let selected = folder.selected_path().cloned();
                    if selected != self.selected {
                        if selected.is_some() {
                            self.display.loading();
                        } else {
                            self.display.clear();
                        }
                    }
                    if self
                        .folder
                        .as_ref()
                        .is_some_and(|old| old.parent() != folder.parent())
                    {
                        self.cache.clear();
                    }
                    self.cache.reconcile(&folder, self.displayed_bytes());
                    self.selected = selected;
                    self.folder = Some(folder);
                }
                Err(error) => self.display.fail(error.to_string()),
            }
        }
        let ready = if let Some(resident) = result.resident {
            Some(match resident.as_ref() {
                Ready::Resident(image) => Ok(Arc::clone(image)),
                Ready::Pixels(pixels) => self.upload(Arc::clone(pixels)),
            })
        } else {
            result.image.map(|image| {
                image
                    .map_err(|error| error.to_string())
                    .and_then(|pixels| self.upload(Arc::new(pixels)))
            })
        };
        if let Some(ready) = ready {
            self.commit_ready(result.generation, result.version, ready);
        } else if matches!(self.display.status, Status::Ready) {
            self.start_prefetch();
        }
    }

    fn upload(&self, pixels: Arc<crate::decode::Pixels>) -> Result<Arc<Image>, String> {
        self.render_state
            .as_ref()
            .ok_or_else(|| "GPU renderer unavailable".to_owned())
            .and_then(|state| Image::upload(state, pixels).map_err(|error| error.to_string()))
    }

    fn commit_ready(
        &mut self,
        generation: u64,
        version: Option<FileVersion>,
        ready: Result<Arc<Image>, String>,
    ) {
        let old = self
            .display
            .image
            .clone()
            .zip(self.displayed_version.clone());
        // Cached and freshly uploaded residents share the atomic admission seam.
        if self
            .display
            .finish(generation, ready, self.selected.clone(), &mut self.view)
        {
            let bytes = self.displayed_bytes();
            if let Some(version) = &version {
                self.cache.admitted(version, bytes);
            }
            if let Some((image, old_version)) = old
                && Some(&old_version) != version.as_ref()
                && self
                    .folder
                    .as_ref()
                    .and_then(|f| f.version(&old_version.path))
                    == Some(&old_version)
            {
                self.cache.insert(
                    Entry {
                        version: old_version,
                        generation: self.displayed_generation,
                        bytes: image.pixels.rgba.len(),
                        value: Arc::new(Ready::Resident(image)),
                        history: true,
                    },
                    bytes,
                );
            }
            self.displayed_version = version;
            self.displayed_generation = generation;
            self.cache.trim(bytes);
            self.start_prefetch();
            let size = self
                .display
                .image
                .as_ref()
                .expect("just committed image")
                .size;
            eprintln!(
                "Displayed {} ({} × {})",
                self.selected
                    .as_deref()
                    .unwrap_or(std::path::Path::new(""))
                    .display(),
                size[0],
                size[1]
            );
        }
    }

    fn displayed_bytes(&self) -> usize {
        self.display
            .image
            .as_ref()
            .map_or(0, |image| image.pixels.rgba.len())
    }

    fn stop_prefetch(&mut self) {
        self.prefetch_epoch = self.prefetch_epoch.saturating_add(1);
        self.prefetch_queue.clear();
        if let Some(loader) = &self.prefetch {
            loader.cancel(self.prefetch_epoch);
        }
    }

    fn start_prefetch(&mut self) {
        self.stop_prefetch();
        if let Some(folder) = &self.folder {
            self.cache.reconcile(folder, self.displayed_bytes());
            self.prefetch_queue = neighbors(folder, self.cache.depth)
                .into_iter()
                .filter(|path| !self.cache.contains(path))
                .collect();
        }
        self.next_prefetch();
    }

    fn next_prefetch(&mut self) {
        if let Some(path) = self.prefetch_queue.pop_front()
            && let Some(loader) = &self.prefetch
        {
            loader.request(Request {
                cached: Vec::new(),
                generation: self.prefetch_epoch,
                path,
                scan_folder: false,
                texture_limit: self.texture_limit,
                sort: self.sort,
                refresh: None,
                load_unchanged: true,
            });
        }
    }

    fn admit_prefetch(&mut self, result: Completion) {
        if result.generation != self.prefetch_epoch || result.image.is_none() {
            return;
        }
        if let Some(version) = result.version
            && self.folder.as_ref().and_then(|f| f.version(&version.path)) == Some(&version)
            && self.selected.as_ref() != Some(&version.path)
            && let Some(Ok(pixels)) = result.image
        {
            let bytes = pixels.rgba.len();
            self.cache.insert(
                Entry {
                    version,
                    generation: result.generation,
                    value: Arc::new(Ready::Pixels(Arc::new(pixels))),
                    bytes,
                    history: false,
                },
                self.displayed_bytes(),
            );
        }
        self.next_prefetch();
    }

    fn desktop_action(&mut self, action: Command) -> Result<(), String> {
        if matches!(action, Command::Reveal) {
            if self.reveal_child.is_some() {
                return Err("Reveal is still running; please wait before retrying.".into());
            }
            let path = self
                .display
                .path
                .as_deref()
                .ok_or("No displayed file to reveal.")?;
            self.reveal_child = Some(crate::desktop::reveal(path)?);
            return Ok(());
        }
        if self.clipboard.is_none() {
            self.clipboard = Some(arboard::Clipboard::new().map_err(|error| {
                format!("Clipboard unavailable; check your desktop session and retry: {error}")
            })?);
        }
        let clipboard = self.clipboard.as_mut().expect("initialized clipboard");
        match action {
            Command::CopyImage => {
                let image = self
                    .display
                    .image
                    .as_ref()
                    .ok_or("No displayed image to copy.")?;
                crate::desktop::copy_image(clipboard, &image.pixels, self.view.transform)
            }
            Command::CopyFile => {
                let path = self
                    .display
                    .path
                    .as_deref()
                    .ok_or("No displayed file to copy.")?;
                clipboard
                    .set()
                    .file_list(&[path])
                    .map_err(|error| format!("Copy File failed; clipboard unavailable: {error}"))
            }
            Command::Paste => {
                let path = crate::desktop::paste(clipboard)?;
                self.open(path);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn dialog(&mut self, frame: &eframe::Frame) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Open image")
            .add_filter("JPEG and PNG", &["jpg", "jpeg", "png"]);
        if let Some(window) = frame.winit_window() {
            dialog = dialog.set_parent(window.as_ref());
        }
        if let Some(parent) = self.selected.as_deref().and_then(|path| path.parent()) {
            dialog = dialog.set_directory(parent);
        }
        if let Some(path) = dialog.pick_file() {
            self.open(path);
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = if let Some(path) = &self.selected {
            let name = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy();
            let position = self
                .folder
                .as_ref()
                .and_then(|folder| {
                    folder
                        .selected
                        .map(|index| format!(" — {}/{}", index + 1, folder.files.len()))
                })
                .unwrap_or_default();
            let state = match &self.display.status {
                Status::Loading => " — Loading".to_owned(),
                Status::Error(error) => format!(" — Error: {}: {error}", path.display()),
                _ => String::new(),
            };
            format!("{name}{position}{state} — glance")
        } else {
            "glance".into()
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }

    fn menus(&self, ui: &mut egui::Ui, command: &mut Option<Command>) {
        ui.menu_button("File", |ui| {
            menu_command(ui, "Open…  ⌘/Ctrl+O", Command::Open, command);
            menu_command(ui, "Paste File", Command::Paste, command);
            ui.add_enabled_ui(self.display.image.is_some(), |ui| {
                menu_command(ui, "Copy Image", Command::CopyImage, command);
                menu_command(ui, "Copy File", Command::CopyFile, command);
                menu_command(ui, "Reveal in File Manager", Command::Reveal, command);
            });
        });
        ui.menu_button("Navigate", |ui| {
            ui.add_enabled_ui(self.folder.is_some(), |ui| {
                for (label, direction) in [
                    ("Previous  ←", Direction::Previous),
                    ("Next  →", Direction::Next),
                    ("First  Home", Direction::First),
                    ("Last  End", Direction::Last),
                ] {
                    menu_command(ui, label, Command::Navigate(direction), command);
                }
                ui.menu_button("Sort", |ui| {
                    for (sort, label) in Sort::ALL {
                        if ui.selectable_label(self.sort == sort, label).clicked() {
                            *command = Some(Command::Sort(sort));
                            ui.close();
                        }
                    }
                });
            });
        });
        ui.menu_button("View", |ui| {
            ui.add_enabled_ui(self.display.image.is_some(), |ui| {
                for (label, action) in [
                    ("Fit  0 / 1", Command::Fit),
                    ("Actual Size (100%)  2", Command::ActualSize),
                    ("Zoom In  +", Command::Zoom(1.25)),
                    ("Zoom Out  −", Command::Zoom(0.8)),
                    (
                        "Rotate Clockwise  R",
                        Command::Transform(Operation::Clockwise),
                    ),
                    (
                        "Rotate Counterclockwise  Shift+R",
                        Command::Transform(Operation::Counterclockwise),
                    ),
                    (
                        "Mirror Horizontally  M",
                        Command::Transform(Operation::Mirror),
                    ),
                    ("Flip Vertically  F", Command::Transform(Operation::Flip)),
                    ("Reset View", Command::Reset),
                ] {
                    menu_command(ui, label, action, command);
                }
            });
        });
        ui.menu_button("Settings", |ui| {
            ui.menu_button("Preload depth", |ui| {
                for depth in 0..=5 {
                    let label = if depth == 0 {
                        "Off".into()
                    } else {
                        depth.to_string()
                    };
                    if ui.radio(self.cache.depth == depth, label).clicked() {
                        *command = Some(Command::Depth(depth));
                        ui.close();
                    }
                }
            });
        });
    }
}

fn menu_command(ui: &mut egui::Ui, label: &str, action: Command, command: &mut Option<Command>) {
    if ui.button(label).clicked() {
        *command = Some(action);
        ui.close();
    }
}

impl eframe::App for Viewer {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(loader) = &self.loader {
            self.admit(loader.take_completion());
        }
        if let Some(prefetch) = &self.prefetch {
            self.admit_prefetch(prefetch.take_completion());
        }
        self.update_title(ctx);
        if let Some(child) = &mut self.reveal_child {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        self.action_error = Some(format!(
                            "Reveal failed ({status}); check the file and system file manager."
                        ));
                    }
                    self.reveal_child = None;
                }
                Ok(None) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
                Err(error) => {
                    self.action_error = Some(format!("Could not check Reveal result: {error}"));
                    self.reveal_child = None;
                }
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let dropped = ctx.input_mut(|input| std::mem::take(&mut input.raw.dropped_files));
        if let Some(path) = dropped.first().map(|file| file.path().to_path_buf()) {
            self.open(path);
        }
        let shortcuts_enabled = !ctx.egui_wants_keyboard_input()
            && !egui::Popup::is_any_open(&ctx)
            && ctx.input(|input| input.focused);
        let mut command = None;
        if shortcuts_enabled {
            if ctx.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::O)) {
                command = Some(Command::Open);
            }
            if ctx.input_mut(|input| input.consume_key(Modifiers::SHIFT, Key::R)) {
                command = Some(Command::Transform(Operation::Counterclockwise));
            }
            for (key, action) in [
                (Key::R, Command::Transform(Operation::Clockwise)),
                (Key::M, Command::Transform(Operation::Mirror)),
                (Key::F, Command::Transform(Operation::Flip)),
                (Key::ArrowLeft, Command::Navigate(Direction::Previous)),
                (Key::ArrowRight, Command::Navigate(Direction::Next)),
                (Key::Home, Command::Navigate(Direction::First)),
                (Key::End, Command::Navigate(Direction::Last)),
                (Key::Num0, Command::Fit),
                (Key::Num1, Command::Fit),
                (Key::Num2, Command::ActualSize),
                (Key::Plus, Command::Zoom(1.25)),
                (Key::Equals, Command::Zoom(1.25)),
                (Key::Minus, Command::Zoom(0.8)),
            ] {
                if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, key)) {
                    command = Some(action);
                }
            }
            if ctx.input_mut(|input| {
                input.consume_key(Modifiers::SHIFT, Key::Plus)
                    || input.consume_key(Modifiers::SHIFT, Key::Equals)
            }) {
                command = Some(Command::Zoom(1.25));
            }
        }
        egui::Panel::top("menus").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| self.menus(ui, &mut command));
        });
        if let Some(error) = &self.action_error {
            let dismiss = egui::Panel::top("action-error")
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(error);
                        ui.button("Dismiss").clicked()
                    })
                    .inner
                })
                .inner;
            if dismiss {
                self.action_error = None;
            }
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).fill(Color32::from_gray(18)))
            .show(ui, |ui| {
                let canvas = ui.available_rect_before_wrap();
                let response = ui.allocate_rect(canvas, Sense::click_and_drag());
                response.context_menu(|ui| {
                    self.menus(ui, &mut command);
                });
                if let Some(image) = &self.display.image {
                    let mut size = self.view.transform.size(image.size);
                    let ppp = ctx.pixels_per_point();
                    if let Some(action) = command {
                        match action {
                            Command::Fit => self.view.fit(),
                            Command::Reset => self.view.reset(),
                            Command::Transform(operation) => self.view.transform(operation),
                            Command::ActualSize => self.view.actual_size(),
                            Command::Zoom(factor) => {
                                self.view.zoom(factor, canvas.center(), canvas, size, ppp)
                            }
                            _ => {}
                        }
                    }
                    size = self.view.transform.size(image.size);
                    if shortcuts_enabled && !egui::Popup::is_any_open(&ctx) {
                        if response.dragged() {
                            self.view.pan(response.drag_delta(), canvas, size, ppp);
                        }
                        if response.hovered()
                            && let Some(pointer) = ctx.input(|i| i.pointer.hover_pos())
                        {
                            ctx.input(|input| {
                                for event in &input.events {
                                    if let egui::Event::MouseWheel {
                                        unit,
                                        delta,
                                        modifiers,
                                        ..
                                    } = event
                                    {
                                        if *unit == egui::MouseWheelUnit::Point
                                            && !modifiers.command
                                            && !modifiers.ctrl
                                        {
                                            self.view.pan(*delta, canvas, size, ppp);
                                        } else {
                                            self.view.zoom(
                                                (delta.y
                                                    * if *unit == egui::MouseWheelUnit::Line {
                                                        0.15
                                                    } else {
                                                        0.01
                                                    })
                                                .exp(),
                                                pointer,
                                                canvas,
                                                size,
                                                ppp,
                                            );
                                        }
                                    }
                                }
                            });
                        }
                    }
                    if canvas.is_positive()
                        && let Some(state) = &self.render_state
                    {
                        ui.painter().add(image.callback(
                            &state.device,
                            canvas,
                            self.view.rect(canvas, size, ppp),
                            self.view.transform,
                        ));
                    }
                } else {
                    let path = self.selected.as_deref().unwrap_or(std::path::Path::new(""));
                    let message = match &self.display.status {
                        Status::Empty if self.folder.is_some() => {
                            "No JPEG or PNG images in this folder.".to_owned()
                        }
                        Status::Empty => "Open an image, or drop a file or folder here.".to_owned(),
                        Status::Loading => format!("Loading {}…", path.display()),
                        Status::Error(error) => format!("{}\n\n{error}", path.display()),
                        Status::Ready => unreachable!(),
                    };
                    let builder = egui::UiBuilder::new().max_rect(canvas).layout(
                        egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    );
                    ui.scope_builder(builder, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(message);
                            if ui.button("Open…").clicked() {
                                command = Some(Command::Open);
                            }
                        });
                    });
                }
            });
        match command {
            Some(Command::Depth(depth)) => {
                self.cache.depth = depth;
                self.cache.trim(self.displayed_bytes());
                if matches!(self.display.status, Status::Ready) {
                    self.start_prefetch();
                } else {
                    self.stop_prefetch();
                }
            }
            Some(Command::Open) => self.dialog(frame),
            Some(Command::Navigate(direction)) => self.refresh(Some(direction)),
            Some(Command::Sort(sort)) => {
                self.sort = sort;
                self.refresh(None);
            }
            Some(
                action
                @ (Command::CopyImage | Command::CopyFile | Command::Paste | Command::Reveal),
            ) => {
                self.action_error = self.desktop_action(action).err();
                ctx.request_repaint();
            }
            _ => {}
        }
        self.update_title(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode::LoadError, folder::tests::TempDir};

    #[test]
    fn prefetched_and_history_hits_use_atomic_admission_without_decode() {
        let dir = TempDir::new();
        let path = dir.0.join("hit.png");
        std::fs::write(&path, b"identity").unwrap();
        for history in [false, true] {
            let version = FileVersion::read(&path).unwrap();
            let resident = Arc::new("display-ready pixels");
            let mut cache = Cache::default();
            cache.insert(
                Entry {
                    version: version.clone(),
                    generation: 1,
                    value: Arc::clone(&resident),
                    bytes: 4,
                    history,
                },
                4,
            );
            let hit = crate::loader::find_cached(cache.snapshot(), Some(&version)).unwrap();
            let mut display = Display::new(Status::Loading);
            display.generation = 9;
            let mut view = View::default();
            assert!(display.finish(9, Ok(hit), Some(path.clone()), &mut view));
            assert!(Arc::ptr_eq(display.image.as_ref().unwrap(), &resident));
            assert!(matches!(display.status, Status::Ready));
            std::fs::write(
                &path,
                if history {
                    b"changed again".as_slice()
                } else {
                    b"changed".as_slice()
                },
            )
            .unwrap();
            let changed = FileVersion::read(&path).unwrap();
            assert!(crate::loader::find_cached(cache.snapshot(), Some(&changed)).is_none());
            // Restore the identity for the next case by using a fresh metadata snapshot.
            if !history {
                std::fs::write(&path, b"identity").unwrap();
            }
        }
    }

    #[test]
    fn swap_holds_prior_pixels_and_framing_until_ready() {
        let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0));
        let mut view = View::default();
        let mut display = Display::new(Status::Empty);
        assert!(display.finish(0, Ok("prior"), None, &mut view));
        view.actual_size();
        view.pan(egui::vec2(37.0, 19.0), canvas, [1600, 800], 1.0);
        let prior_rect = view.rect(canvas, [1600, 800], 1.0);
        display.generation = 1;
        display.loading();
        assert_eq!(display.image, Some("prior"));
        assert_eq!(view.rect(canvas, [1600, 800], 1.0), prior_rect);
        assert!(matches!(display.status, Status::Loading));

        assert!(display.finish(1, Ok("next uploaded resident"), None, &mut view));
        assert_eq!(display.image, Some("next uploaded resident"));
        assert!(matches!(display.status, Status::Ready));
        assert_eq!(
            view.rect(canvas, [1600, 800], 1.0).center(),
            canvas.center()
        );
        assert_eq!(view.scale(canvas, [1600, 800], 1.0), 0.25);
    }

    #[test]
    fn superseded_swap_is_dropped_without_display_or_reset() {
        let mut view = View::default();
        view.actual_size();
        let prior = Arc::new("prior");
        let superseded = Arc::new("superseded");
        let weak = Arc::downgrade(&superseded);
        let mut display = Display {
            image: Some(Arc::clone(&prior)),
            status: Status::Loading,
            generation: 3,
            path: None,
        };
        assert!(!display.finish(2, Ok(superseded), None, &mut view));
        assert!(weak.upgrade().is_none());
        assert!(Arc::ptr_eq(display.image.as_ref().unwrap(), &prior));
        assert!(matches!(display.status, Status::Loading));
        let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 100.0));
        assert_eq!(view.scale(canvas, [1000, 1000], 1.0), 1.0);
        assert!(!display.finish(2, Err("stale failure".into()), None, &mut view));
        assert!(matches!(display.status, Status::Loading));
        assert!(display.finish(3, Ok(Arc::new("latest")), None, &mut view));
        assert_eq!(**display.image.as_ref().unwrap(), "latest");
    }

    #[test]
    fn decode_or_upload_error_retains_prior_and_allows_next_swap() {
        let mut view = View::default();
        let mut display = Display::new(Status::Empty);
        assert!(display.finish(0, Ok("prior"), None, &mut view));
        for error in ["decode failed", "GPU upload failed"] {
            view.actual_size();
            display.generation += 1;
            display.loading();
            assert!(!display.finish(display.generation, Err(error.into()), None, &mut view));
            assert_eq!(display.image, Some("prior"));
            assert!(matches!(&display.status, Status::Error(message) if message == error));
            let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 100.0));
            assert_eq!(view.scale(canvas, [1000, 1000], 1.0), 1.0);
        }
        display.generation += 1;
        display.loading();
        assert!(display.finish(display.generation, Ok("recovered"), None, &mut view));
        assert_eq!(display.image, Some("recovered"));
    }

    #[test]
    fn transformed_resident_persists_until_different_file_swap() {
        let mut display = Display::new(Status::Empty);
        let mut view = View::default();
        let first = Some(PathBuf::from("/tmp/first.png"));
        let second = Some(PathBuf::from("/tmp/second.png"));
        assert!(display.finish(0, Ok("first pixels"), first.clone(), &mut view));
        view.transform(Operation::Clockwise);
        view.actual_size();
        display.generation = 1;
        display.loading();
        view.transform(Operation::Mirror);
        let transform = view.transform;
        assert_eq!(display.path, first);
        assert_eq!(display.image, Some("first pixels"));
        assert!(!display.finish(0, Ok("stale pixels"), second.clone(), &mut view));
        assert_eq!(view.transform, transform);
        assert!(!display.finish(1, Err("load failed".into()), second.clone(), &mut view));
        assert_eq!(view.transform, transform);
        assert_eq!(display.path, first);
        display.generation = 2;
        assert!(display.finish(2, Ok("second pixels"), second.clone(), &mut view));
        assert_eq!(display.path, second);
        assert_eq!(view.transform, crate::transform::Transform::default());
        let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0));
        assert_eq!(view.scale(canvas, [1600, 800], 1.0), 0.25);
    }

    #[test]
    fn same_file_reload_preserves_transforms_zoom_and_pan() {
        let mut display = Display::new(Status::Empty);
        let mut view = View::default();
        let path = Some(PathBuf::from("/tmp/same.png"));
        assert!(display.finish(0, Ok("prior pixels"), path.clone(), &mut view));
        view.transform(Operation::Flip);
        view.actual_size();
        let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0));
        view.pan(egui::vec2(37.0, 19.0), canvas, [1600, 800], 1.0);
        let prior_rect = view.rect(canvas, [1600, 800], 1.0);
        let transform = view.transform;
        display.generation = 1;
        display.loading();
        assert!(display.finish(1, Ok("refreshed pixels"), path, &mut view));
        assert_eq!(view.transform, transform);
        assert_eq!(view.rect(canvas, [1600, 800], 1.0), prior_rect);
    }

    #[test]
    fn cold_open_and_empty_folder_have_no_resident() {
        let mut display = Display::new(Status::Empty);
        display.loading();
        assert!(display.image.is_none());
        display.fail("missing file".into());
        assert!(display.image.is_none());
        let mut view = View::default();
        assert!(display.finish(0, Ok("image"), None, &mut view));
        display.clear();
        assert!(display.image.is_none());
        assert!(matches!(display.status, Status::Empty));
    }

    #[test]
    fn stale_results_are_rejected_and_error_keeps_folder() {
        let dir = TempDir::new();
        let path = dir.0.join("latest.png");
        let mut viewer = Viewer {
            prefetch: None,
            cache: Cache::default(),
            displayed_version: None,
            displayed_generation: 0,
            prefetch_epoch: 0,
            prefetch_queue: Default::default(),
            loader: None,
            folder: None,
            selected: Some(path.clone()),
            texture_limit: 8192,
            display: Display {
                generation: 3,
                ..Display::new(Status::Loading)
            },
            last_title: String::new(),
            sort: Sort::default(),
            view: View::default(),
            render_state: None,
            clipboard: None,
            action_error: None,
            reveal_child: None,
        };
        viewer.admit(Completion {
            generation: 2,
            image: Some(Err(LoadError::Unsupported)),
            ..Default::default()
        });
        assert!(matches!(viewer.display.status, Status::Loading));
        viewer.admit(Completion {
            generation: 3,
            folder: Some(Ok(Folder::open(path.clone()).unwrap())),
            image: Some(Err(LoadError::Unsupported)),
            ..Default::default()
        });
        assert!(matches!(viewer.display.status, Status::Error(_)));
        assert_eq!(viewer.folder.as_ref().unwrap().selected_path(), Some(&path));
        viewer.update_title(&egui::Context::default());
        assert!(viewer.last_title.contains(&path.display().to_string()));
        assert!(viewer.last_title.contains("Error:"));
    }
}
