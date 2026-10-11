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

    fn finish_tiles(
        &mut self,
        generation: u64,
        ready: T,
        coverage: Option<crate::tiles::CoverageReady>,
        path: Option<PathBuf>,
        view: &mut View,
    ) -> bool {
        if coverage.is_none() {
            return false;
        }
        self.finish(generation, Ok(ready), path, view)
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
    Playback,
    NextFrame,
    Depth(usize),
    Rate(crate::animation::Rate),
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
    SetDefaultHandler,
}

#[derive(Default)]
struct SpacePan {
    held: bool,
    dragging: bool,
    // A released temporary gesture must not fall through to ordinary left-drag pan.
    cancelled_drag: bool,
}

impl SpacePan {
    fn update_key(&mut self, pressed: bool, down: bool, enabled: bool, pointer_down: bool) {
        if !pointer_down {
            self.dragging = false;
            self.cancelled_drag = false;
        }
        if !enabled || !down {
            self.held = false;
            self.cancelled_drag |= self.dragging;
            self.dragging = false;
        } else if pressed {
            self.held = true;
        }
    }

    fn active(&self, enlarged: bool) -> bool {
        self.held && enlarged && !self.cancelled_drag
    }

    fn pan_drag(&mut self, dragged: bool, enlarged: bool) -> bool {
        self.dragging = dragged && self.active(enlarged);
        dragged && !self.cancelled_drag
    }

    fn pan_delta(&mut self, response: &egui::Response, enlarged: bool) -> egui::Vec2 {
        let dragged = if self.held {
            response.dragged_by(egui::PointerButton::Primary)
        } else {
            response.dragged()
        };
        let drag_pan = self.pan_drag(dragged, enlarged);
        // Choose one delta, even when Space and a button drag are both active.
        if self.active(enlarged) && response.hovered() {
            response.ctx.input(|input| input.pointer.delta())
        } else if drag_pan {
            response.drag_delta()
        } else {
            egui::Vec2::ZERO
        }
    }

    fn cursor(&self, enlarged: bool, moving: bool) -> Option<egui::CursorIcon> {
        if !enlarged || self.cancelled_drag {
            None
        } else if moving {
            Some(egui::CursorIcon::Grabbing)
        } else if self.held {
            Some(egui::CursorIcon::Grab)
        } else {
            None
        }
    }
}

fn shortcuts_enabled(ctx: &egui::Context) -> bool {
    !ctx.egui_wants_keyboard_input()
        && !egui::Popup::is_any_open(ctx)
        && ctx.input(|input| input.focused)
}

struct Playback {
    worker: crate::animation::Worker,
    clock: crate::animation::Clock,
    origin: std::time::Instant,
    waiting: bool,
    manual: bool,
    final_frame: bool,
    displayed_index: usize,
    candidate_index: usize,
    resync: bool,
}

impl Playback {
    fn set_rate(&mut self, encoded: std::time::Duration, rate: crate::animation::Rate) {
        self.clock.set_rate(self.origin.elapsed(), encoded, rate);
        // Invalidate work scheduled under the old policy; continue from the displayed
        // position even if late-frame omission moved the decoder ahead of it.
        self.worker.cancel();
        self.waiting = false;
        self.manual = false;
        self.resync = true;
    }

    fn new(
        version: FileVersion,
        context: egui::Context,
        duration: std::time::Duration,
    ) -> Result<Self, crate::decode::LoadError> {
        Ok(Self {
            worker: crate::animation::Worker::new(version, context)?,
            clock: crate::animation::Clock::new(
                std::time::Duration::ZERO,
                crate::animation::Rate::default().interval(duration),
            ),
            origin: std::time::Instant::now(),
            waiting: false,
            manual: false,
            final_frame: false,
            displayed_index: 0,
            candidate_index: 0,
            resync: false,
        })
    }
}

pub struct Viewer {
    context: egui::Context,
    animation: Option<Playback>,
    svg_worker: Option<crate::decode::svg::worker::Worker>,
    pending_svg: Option<(u64, Arc<Image>)>,
    pending_animation: bool,
    uploads: Arc<std::sync::Mutex<crate::render::Uploads>>,
    staging_frame: u64,
    pending_image: Option<(u64, Option<FileVersion>, Arc<Image>)>,
    last_paint: Option<egui::PaintCallback>,
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
    settings: crate::settings::Settings,
    settings_store: crate::settings::Store,
    sort: Sort,
    view: View,
    space_pan: SpacePan,
    render_state: Option<eframe::egui_wgpu::RenderState>,
    clipboard: Option<arboard::Clipboard>,
    action_error: Option<String>,
    action_notice: Option<String>,
    default_handler_result: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
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
        let (settings, settings_store) = crate::settings::Store::load();
        let mut cache = Cache::default();
        cache.depth = settings.preload_depth;
        let mut viewer = Self {
            context: cc.egui_ctx.clone(),
            animation: None,
            svg_worker: None,
            pending_svg: None,
            pending_animation: false,
            uploads: crate::render::Uploads::new(cc.wgpu_render_state.as_ref()),
            staging_frame: 0,
            pending_image: None,
            last_paint: None,
            prefetch,
            cache,
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
            settings,
            settings_store,
            sort: Sort::default(),
            view: View::default(),
            space_pan: SpacePan::default(),
            render_state: cc.wgpu_render_state.clone(),
            clipboard: None,
            action_error: prefetch_error,
            action_notice: None,
            default_handler_result: None,
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
        self.pending_svg = None;
        if let Some(worker) = &self.svg_worker {
            worker.cancel();
        }
        if matches!(self.display.status, Status::Loading) {
            self.pending_image = None;
            self.animation = None;
            self.pending_animation = false;
        }
        self.stop_prefetch();
        let Some(generation) = self.display.generation.checked_add(1) else {
            self.display
                .fail("Selection counter exhausted; restart glance.".into());
            return;
        };
        self.display.generation = generation;
        if let Some((old_generation, _, _)) = &mut self.pending_image {
            *old_generation = generation;
        }
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
                        self.animation = None;
                        self.pending_animation = false;
                        self.pending_image = None;
                        if selected.is_some() {
                            self.display.loading();
                        } else {
                            self.display.clear();
                            self.last_paint = None;
                            self.pending_image = None;
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
            self.animation = None;
            self.pending_animation = false;
            match ready {
                Ok(image) => self.pending_image = Some((result.generation, result.version, image)),
                Err(error) => {
                    self.display.fail(error);
                    self.pending_image = None;
                }
            }
        } else if matches!(self.display.status, Status::Ready) {
            self.start_prefetch();
        }
    }

    fn upload(&self, pixels: Arc<crate::decode::Pixels>) -> Result<Arc<Image>, String> {
        self.render_state
            .as_ref()
            .ok_or_else(|| "GPU renderer unavailable".to_owned())
            .and_then(|state| {
                Image::upload(state, pixels, Arc::clone(&self.uploads))
                    .map_err(|error| error.to_string())
            })
    }

    fn commit_ready(
        &mut self,
        generation: u64,
        version: Option<FileVersion>,
        ready: Arc<Image>,
        coverage: crate::tiles::CoverageReady,
    ) {
        if self.pending_animation {
            self.pending_animation = false;
            if self.display.finish_tiles(
                generation,
                ready,
                Some(coverage),
                self.selected.clone(),
                &mut self.view,
            ) && let Some(playback) = &mut self.animation
            {
                playback.waiting = false;
                playback.displayed_index = playback.candidate_index;
                if std::env::var_os("GLANCE_TRACE_ANIMATION").is_some_and(|v| v == "1") {
                    eprintln!("Animation frame {} admitted", playback.displayed_index);
                }
                if playback.final_frame && !playback.manual {
                    playback.clock.step();
                }
                playback.manual = false;
            }
            return;
        }
        let old = self
            .display
            .image
            .clone()
            .zip(self.displayed_version.clone());
        // Cached and freshly uploaded residents share the atomic admission seam.
        if self.display.finish_tiles(
            generation,
            ready,
            Some(coverage),
            self.selected.clone(),
            &mut self.view,
        ) {
            let bytes = self.displayed_bytes();
            if let Some(version) = &version {
                self.cache.admitted(version, bytes);
            }
            if let Some((image, old_version)) = old
                && image.pixels.animation_delay.is_none()
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
                        bytes: image.pixels.accounted_bytes(),
                        value: Arc::new(Ready::Pixels(Arc::clone(&image.pixels))),
                        history: true,
                    },
                    bytes,
                );
            }
            self.displayed_version = version;
            self.displayed_generation = generation;
            self.animation = None;
            if let Some(duration) = self
                .display
                .image
                .as_ref()
                .and_then(|image| image.pixels.animation_delay)
                && let Some(version) = self.displayed_version.clone()
            {
                match Playback::new(version, self.context.clone(), duration) {
                    Ok(mut playback) => {
                        playback.clock.set_rate(
                            std::time::Duration::ZERO,
                            duration,
                            self.settings.animation_rate,
                        );
                        self.animation = Some(playback);
                    }
                    Err(error) => self.display.fail(error.to_string()),
                }
            }
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

    fn set_animation_rate(&mut self, rate: crate::animation::Rate) {
        if self.settings.animation_rate == rate {
            return;
        }
        self.settings.animation_rate = rate;
        self.settings_store.save(self.settings);
        if let Some(playback) = &mut self.animation
            && let Some(encoded) = self
                .display
                .image
                .as_ref()
                .and_then(|image| image.pixels.animation_delay)
        {
            playback.set_rate(encoded, rate);
            if self.pending_animation {
                self.pending_animation = false;
                self.pending_image = None;
            }
        }
        self.context.request_repaint();
    }

    fn tick_animation(&mut self, ctx: &egui::Context) {
        let Some(playback) = &mut self.animation else {
            return;
        };
        let now = playback.origin.elapsed();
        if (playback.clock.paused || playback.clock.rate == crate::animation::Rate::Off)
            && !playback.manual
        {
            return;
        }
        if self.pending_animation {
            ctx.request_repaint();
            return;
        }
        if let Some(result) = playback.worker.take() {
            match result {
                Err(error) => {
                    self.display.fail(error.to_string());
                    self.animation = None;
                    return;
                }
                Ok(None) => {
                    playback.waiting = false;
                    playback.clock.step();
                    return;
                }
                Ok(Some(frame)) => {
                    let duration = playback.clock.rate.interval(
                        frame
                            .pixels
                            .animation_delay
                            .expect("animation frame timing"),
                    );
                    let skip = if playback.manual {
                        playback.clock.stepped(duration);
                        false
                    } else {
                        playback.clock.arrive(now, duration, frame.final_frame)
                    };
                    playback.final_frame = frame.final_frame;
                    playback.candidate_index = frame.index;
                    if skip {
                        playback.worker.advance(None);
                        return;
                    }
                    match self.upload(frame.pixels) {
                        Ok(image) => {
                            self.pending_animation = true;
                            self.pending_image = Some((
                                self.display.generation,
                                self.displayed_version.clone(),
                                image,
                            ));
                            ctx.request_repaint();
                        }
                        Err(error) => {
                            self.display.fail(error);
                            self.animation = None;
                        }
                    }
                    return;
                }
            }
        }
        if !playback.waiting && playback.clock.due(now) {
            playback.waiting = true;
            playback
                .worker
                .advance(playback.resync.then_some(playback.displayed_index));
            playback.resync = false;
        }
        if !playback.waiting {
            ctx.request_repaint_after(playback.clock.deadline.saturating_sub(now));
        }
    }

    fn tick_svg(&mut self, canvas: egui::Rect, ppp: f32) {
        if !canvas.is_positive()
            || !matches!(self.display.status, Status::Ready)
            || self.display.path != self.selected
            || self.pending_image.is_some()
        {
            return;
        }
        let Some(image) = self.display.image.clone() else {
            return;
        };
        let Some(document) = &image.pixels.svg else {
            return;
        };
        if self.svg_worker.is_none() {
            match crate::decode::svg::worker::Worker::new(self.context.clone()) {
                Ok(worker) => self.svg_worker = Some(worker),
                Err(error) => {
                    self.action_error = Some(format!("SVG raster worker unavailable: {error}"));
                    return;
                }
            }
        }
        let size = self.view.transform.size(image.size);
        let needed = self.view.scale(canvas, size, ppp);
        let worker = self.svg_worker.as_ref().expect("SVG worker initialized");
        worker.update(&image.pixels, needed, self.view.exact_svg_scale());
        if self
            .pending_svg
            .as_ref()
            .is_some_and(|(revision, _)| !worker.current(*revision, document))
        {
            self.pending_svg = None;
        }
        if let Some((revision, result)) = worker.take() {
            match result {
                Ok(pixels) if worker.current(revision, document) => {
                    match self.upload(Arc::new(pixels)) {
                        Ok(image) => self.pending_svg = Some((revision, image)),
                        Err(error) => self.action_error = Some(error),
                    }
                }
                Err(error) => self.action_error = Some(error.to_string()),
                _ => {}
            }
        }
        let Some((revision, candidate)) = self.pending_svg.take() else {
            return;
        };
        let prepared = self
            .render_state
            .as_ref()
            .ok_or_else(|| "GPU renderer unavailable".to_owned())
            .and_then(|state| {
                candidate
                    .callback(
                        state,
                        canvas,
                        self.view.rect(canvas, size, ppp),
                        self.view.transform,
                        needed,
                        self.staging_frame,
                    )
                    .map_err(|e| e.to_string())
            });
        match prepared {
            Ok(Some(paint)) => {
                // Same identity and framing, but still require complete GPU coverage.
                if self.display.finish_tiles(
                    self.display.generation,
                    candidate,
                    Some(paint.coverage),
                    self.display.path.clone(),
                    &mut self.view,
                ) {
                    self.last_paint = Some(paint.paint);
                    let bytes = self.displayed_bytes();
                    if let Some(version) = &self.displayed_version {
                        self.cache.admitted(version, bytes);
                    }
                    self.cache.trim(bytes);
                }
            }
            Ok(None) => {
                self.pending_svg = Some((revision, candidate));
                self.context.request_repaint();
            }
            Err(error) => self.action_error = Some(error),
        }
    }

    fn displayed_bytes(&self) -> usize {
        self.display
            .image
            .as_ref()
            .map_or(0, |image| image.pixels.accounted_bytes())
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
            self.prefetch_queue = neighbors(folder, self.settings.preload_depth)
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
            let bytes = pixels.accounted_bytes();
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
                if let Some(svg) = &image.pixels.svg {
                    let intrinsic = svg.rasterize(1.0).map_err(|e| e.to_string())?;
                    crate::desktop::copy_image(clipboard, &intrinsic, self.view.transform)
                } else {
                    crate::desktop::copy_image(clipboard, &image.pixels, self.view.transform)
                }
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

    fn set_default_handler(&mut self, frame: &eframe::Frame) {
        if self.default_handler_result.is_some() {
            self.action_error = Some("Setting defaults is still running; please wait.".into());
            return;
        }
        self.space_pan.update_key(false, false, false, true);
        let mut dialog = rfd::MessageDialog::new()
            .set_title("Make Glance the default image viewer?")
            .set_description("Change the default application for all image types supported by Glance? This replaces your current image viewer associations.")
            .set_buttons(rfd::MessageButtons::OkCancel);
        if let Some(window) = frame.winit_window() {
            dialog = dialog.set_parent(window.as_ref());
        }
        if dialog.show() != rfd::MessageDialogResult::Ok {
            return;
        }
        self.action_notice = None;
        self.action_error = None;
        let (sender, receiver) = std::sync::mpsc::channel();
        match std::thread::Builder::new()
            .name("default-handler".into())
            .spawn(move || {
                let _ = sender.send(crate::default_handler::set_default_handler());
            }) {
            Ok(_) => self.default_handler_result = Some(receiver),
            Err(error) => {
                self.action_error =
                    Some(format!("Cannot start default application worker: {error}"))
            }
        }
        self.context.request_repaint();
    }

    fn dialog(&mut self, frame: &eframe::Frame) {
        self.space_pan.update_key(false, false, false, true);
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
                Status::Ready => self
                    .display
                    .image
                    .as_ref()
                    .and_then(|image| image.pixels.warning.as_ref())
                    .map_or_else(String::new, |warning| format!(" — {warning}")),
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
        ui.menu_button("Playback", |ui| {
            ui.add_enabled_ui(self.animation.is_some(), |ui| {
                let paused = self
                    .animation
                    .as_ref()
                    .is_some_and(|p| p.clock.paused || p.clock.rate == crate::animation::Rate::Off);
                ui.add_enabled_ui(
                    self.settings.animation_rate != crate::animation::Rate::Off,
                    |ui| {
                        menu_command(
                            ui,
                            if paused { "Resume  P" } else { "Pause  P" },
                            Command::Playback,
                            command,
                        );
                    },
                );
                menu_command(ui, "Next Frame  .", Command::NextFrame, command);
            });
        });
        ui.menu_button("Settings", |ui| {
            ui.add_enabled_ui(self.default_handler_result.is_none(), |ui| {
                menu_command(
                    ui,
                    "Set as the default image viewer…",
                    Command::SetDefaultHandler,
                    command,
                );
            });
            ui.separator();
            ui.menu_button("Animation rate", |ui| {
                animation_rate_menu(ui, self.settings.animation_rate, command);
            });
            ui.menu_button("Preload depth", |ui| {
                for depth in 0..=5 {
                    let label = if depth == 0 {
                        "Off".into()
                    } else {
                        depth.to_string()
                    };
                    if ui
                        .radio(self.settings.preload_depth == depth, label)
                        .clicked()
                    {
                        *command = Some(Command::Depth(depth));
                        ui.close();
                    }
                }
            });
        });
    }
}

// Run after constructing menus so focus acquired this frame also suppresses close.
fn close_shortcut(ctx: &egui::Context) {
    if shortcuts_enabled(ctx)
        && ctx.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::W))
    {
        // eframe turns this into the same Close viewport event as the OS close button.
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

fn keyboard_command(ctx: &egui::Context) -> Option<Command> {
    let mut command = None;
    if ctx.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::O)) {
        command = Some(Command::Open);
    }
    if ctx.input_mut(|input| input.consume_key(Modifiers::SHIFT, Key::R)) {
        command = Some(Command::Transform(Operation::Counterclockwise));
    }
    for (key, action) in [
        (Key::P, Command::Playback),
        (Key::Period, Command::NextFrame),
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
    command
}

fn animation_rate_options(
    active: crate::animation::Rate,
) -> impl Iterator<Item = (crate::animation::Rate, &'static str, bool)> {
    crate::animation::Rate::ALL
        .into_iter()
        .map(move |(rate, label, _)| (rate, label, active == rate))
}

fn animation_rate_menu(
    ui: &mut egui::Ui,
    active: crate::animation::Rate,
    command: &mut Option<Command>,
) {
    for (rate, label, selected) in animation_rate_options(active) {
        if ui.radio(selected, label).clicked() {
            *command = Some(Command::Rate(rate));
            ui.close();
        }
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
        self.tick_animation(ctx);
        self.update_title(ctx);
        if let Some(receiver) = &self.default_handler_result {
            match receiver.try_recv() {
                Ok(result) => {
                    match result {
                        Ok(()) => self.action_notice = Some(
                            "Glance is now the default image viewer for its supported image types."
                                .into(),
                        ),
                        Err(error) => self.action_error = Some(error),
                    }
                    self.default_handler_result = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100))
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.action_error = Some("Default application worker stopped; some associations may have changed. Check your system defaults before retrying.".into());
                    self.default_handler_result = None;
                }
            }
        }
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
        let shortcuts_enabled = shortcuts_enabled(&ctx);
        ctx.input(|input| {
            let pressed = input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: Key::Space,
                        pressed: true,
                        repeat: false,
                        ..
                    }
                )
            });
            self.space_pan.update_key(
                pressed,
                input.key_down(Key::Space),
                shortcuts_enabled,
                input.pointer.primary_down(),
            );
        });
        let mut command = shortcuts_enabled.then(|| keyboard_command(&ctx)).flatten();
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
        if let Some(notice) = &self.action_notice {
            let dismiss = egui::Panel::top("action-notice")
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(notice);
                        ui.button("Dismiss").clicked()
                    })
                    .inner
                })
                .inner;
            if dismiss {
                self.action_notice = None;
            }
        }
        self.staging_frame = self.staging_frame.wrapping_add(1);
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).fill(Color32::from_gray(18)))
            .show(ui, |ui| {
                let canvas = ui.available_rect_before_wrap();
                let response = ui.allocate_rect(canvas, Sense::click_and_drag());
                response.context_menu(|ui| {
                    self.menus(ui, &mut command);
                });
                // Menus can acquire focus in this frame, after the initial key guard.
                if !crate::app::shortcuts_enabled(&ctx) {
                    self.space_pan.update_key(false, false, false, true);
                }
                // Stage against the framing that will actually be committed. The
                // previous image/view remain intact until every visible tile is ready.
                if (!self.pending_animation
                    || self
                        .animation
                        .as_ref()
                        .is_some_and(|p| !p.clock.paused || p.manual))
                    && let Some((generation, version, image)) = self.pending_image.take()
                    && generation == self.display.generation
                {
                    let mut candidate_view = self.view.clone();
                    if self.display.path != self.selected {
                        candidate_view.reset();
                    }
                    let size = candidate_view.transform.size(image.size);
                    let ppp = ctx.pixels_per_point();
                    let prepared = self
                        .render_state
                        .as_ref()
                        .ok_or_else(|| "GPU renderer unavailable".to_owned())
                        .and_then(|state| {
                            image
                                .callback(
                                    state,
                                    canvas,
                                    candidate_view.rect(canvas, size, ppp),
                                    candidate_view.transform,
                                    candidate_view.scale(canvas, size, ppp),
                                    self.staging_frame,
                                )
                                .map_err(|e| e.to_string())
                        });
                    match prepared {
                        Ok(Some(paint)) => {
                            self.commit_ready(generation, version, image, paint.coverage);
                            self.last_paint = Some(paint.paint);
                        }
                        Ok(None) => {
                            self.pending_image = Some((generation, version, image));
                            ctx.request_repaint();
                        }
                        Err(error) => {
                            self.display.fail(error);
                            if self.pending_animation {
                                self.pending_animation = false;
                                self.animation = None;
                            }
                        }
                    }
                }
                if let Some(image) = self.display.image.clone() {
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
                        let enlarged = self.view.enlarged(canvas, size, ppp);
                        let delta = self.space_pan.pan_delta(&response, enlarged);
                        self.view.pan(delta, canvas, size, ppp);
                        let moving = delta != egui::Vec2::ZERO || response.dragged();
                        if (response.hovered() || response.dragged())
                            && let Some(cursor) = self.space_pan.cursor(enlarged, moving)
                        {
                            ctx.set_cursor_icon(cursor);
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
                    self.tick_svg(canvas, ppp);
                    let image = self.display.image.as_ref().expect("displayed image");
                    if canvas.is_positive()
                        && let Some(state) = &self.render_state
                    {
                        match image.callback(
                            state,
                            canvas,
                            self.view.rect(canvas, size, ppp),
                            self.view.transform,
                            self.view.scale(canvas, size, ppp),
                            self.staging_frame,
                        ) {
                            Ok(Some(paint)) => self.last_paint = Some(paint.paint),
                            Ok(None) => ctx.request_repaint(),
                            Err(error) => self.action_error = Some(error.to_string()),
                        }
                        if let Some(paint) = &self.last_paint {
                            ui.painter().add(paint.clone());
                        }
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
        close_shortcut(&ctx);
        match command {
            Some(Command::Depth(depth)) => {
                if self.settings.preload_depth == depth {
                    return;
                }
                self.settings.preload_depth = depth;
                self.settings_store.save(self.settings);
                // Cache depth is derived policy, not a second preference source.
                self.cache.depth = self.settings.preload_depth;
                self.cache.trim(self.displayed_bytes());
                if matches!(self.display.status, Status::Ready) {
                    self.start_prefetch();
                } else {
                    self.stop_prefetch();
                }
            }
            Some(Command::Rate(rate)) => self.set_animation_rate(rate),
            Some(Command::Playback) => {
                if let Some(playback) = &mut self.animation {
                    if playback.clock.rate == crate::animation::Rate::Off {
                        return;
                    }
                    playback.clock.toggle(playback.origin.elapsed());
                    ctx.request_repaint();
                    // Pause holds an uncommitted candidate without changing the displayed frame.
                    // Keep that candidate resident for Resume/Next Frame rather than consuming another frame.
                }
            }
            Some(Command::NextFrame) => {
                if let Some(playback) = &mut self.animation {
                    playback.clock.step();
                    playback.manual = true;
                    playback.resync = false;
                    ctx.request_repaint();
                    playback.waiting = true;
                    self.pending_image = None;
                    self.pending_animation = false;
                    playback.worker.advance(Some(playback.displayed_index));
                }
            }
            Some(Command::SetDefaultHandler) => self.set_default_handler(frame),
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
    fn rate_menu_has_one_active_option_and_selects_via_keyboard() {
        for (active, _, _) in crate::animation::Rate::ALL {
            let options: Vec<_> = animation_rate_options(active).collect();
            assert_eq!(options.len(), 7);
            assert_eq!(options.iter().filter(|option| option.2).count(), 1);
            assert!(options.iter().any(|option| option.0 == active && option.2));
        }
        let ctx = egui::Context::default();
        let mut command = None;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            animation_rate_menu(ui, crate::animation::Rate::Exact, &mut command);
        });
        output.textures_delta.clear();
        assert!(command.is_none());
        for key in [Key::Tab, Key::Enter] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![key_event(key, false)],
                    ..Default::default()
                },
                |ui| {
                    animation_rate_menu(ui, crate::animation::Rate::Exact, &mut command);
                },
            );
            output.textures_delta.clear();
        }
        assert!(matches!(
            command,
            Some(Command::Rate(crate::animation::Rate::HonorFile))
        ));
    }

    #[test]
    fn animation_shortcuts_and_same_path_frame_swaps_preserve_view_and_copy() {
        for (key, step) in [(Key::P, false), (Key::Period, true)] {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![key_event(key, false)],
                    ..Default::default()
                },
                |ui| {
                    assert!(matches!(
                        (keyboard_command(ui.ctx()), step),
                        (Some(Command::Playback), false) | (Some(Command::NextFrame), true)
                    ));
                },
            );
            output.textures_delta.clear();
        }
        let path = Some(PathBuf::from("animation.gif"));
        let mut display = Display::new(Status::Empty);
        let mut view = View::default();
        let pixels = |red| {
            Arc::new(crate::decode::Pixels {
                size: [2, 1],
                rgba: vec![red, 0, 0, 255, 0, red, 0, 128],
                animation_delay: Some(std::time::Duration::from_millis(30)),
                ..Default::default()
            })
        };
        assert!(display.finish(0, Ok(pixels(100)), path.clone(), &mut view));
        view.transform(Operation::Mirror);
        view.actual_size();
        let transform = view.transform;
        let mut clock = crate::animation::Clock::new(
            std::time::Duration::ZERO,
            std::time::Duration::from_millis(30),
        );
        clock.toggle(std::time::Duration::from_millis(10));
        for ms in [30, 100, 1000] {
            assert!(!clock.due(std::time::Duration::from_millis(ms)));
            assert_eq!(display.image.as_ref().unwrap().rgba[0], 100);
        }
        assert!(display.finish(0, Ok(pixels(200)), path, &mut view));
        assert_eq!(view.transform, transform);
        let copy = transform
            .materialize(display.image.as_ref().unwrap())
            .unwrap();
        assert_eq!(copy.rgba, [0, 200, 0, 128, 200, 0, 0, 255]);
        assert!(!display.finish(0, Err("truncated animation".into()), None, &mut view));
        assert_eq!(display.image.as_ref().unwrap().rgba[0], 200);
    }

    #[test]
    fn canvas_drag_with_and_without_space_moves_image() {
        for space in [false, true] {
            let ctx = egui::Context::default();
            let mut pan = SpacePan::default();
            let mut view = View::default();
            view.actual_size();
            let start = egui::pos2(100.0, 100.0);
            let end = start + egui::vec2(40.0, -20.0);
            let mut before = egui::Pos2::ZERO;
            for step in 0..5 {
                let events = match step {
                    0 => vec![egui::Event::PointerMoved(start)],
                    1 if space => vec![key_event(Key::Space, false)],
                    2 => vec![egui::Event::PointerButton {
                        pos: start,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    }],
                    3 => vec![egui::Event::PointerMoved(end)],
                    4 => vec![egui::Event::PointerMoved(end + egui::vec2(15.0, 10.0))],
                    _ => vec![],
                };
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(400.0, 300.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let enabled = shortcuts_enabled(&ctx);
                        ctx.input(|input| {
                            pan.update_key(
                                input.events.iter().any(|event| {
                                    matches!(
                                        event,
                                        egui::Event::Key {
                                            key: Key::Space,
                                            pressed: true,
                                            repeat: false,
                                            ..
                                        }
                                    )
                                }),
                                input.key_down(Key::Space),
                                enabled,
                                input.pointer.primary_down(),
                            )
                        });
                        egui::CentralPanel::default().show(ui, |ui| {
                            let canvas = ui.available_rect_before_wrap();
                            let response = ui.allocate_rect(canvas, Sense::click_and_drag());
                            response.context_menu(|ui| {
                                let _ = ui.button("Open…");
                            });
                            if !shortcuts_enabled(&ctx) {
                                pan.update_key(false, false, false, true);
                            }
                            let enlarged = view.enlarged(canvas, [1600, 1000], 1.0);
                            let delta = if enabled && !egui::Popup::is_any_open(&ctx) {
                                pan.pan_delta(&response, enlarged)
                            } else {
                                egui::Vec2::ZERO
                            };
                            view.pan(delta, canvas, [1600, 1000], 1.0);
                            if (response.hovered() || response.dragged())
                                && let Some(cursor) = pan.cursor(
                                    enlarged,
                                    delta != egui::Vec2::ZERO || response.dragged(),
                                )
                            {
                                ctx.set_cursor_icon(cursor);
                            }
                            // Hit testing uses the previous frame's widget rects.
                            if step == 1 {
                                assert_eq!(
                                    ctx.output(|out| out.cursor_icon),
                                    if space {
                                        egui::CursorIcon::Grab
                                    } else {
                                        egui::CursorIcon::Default
                                    }
                                );
                            }
                            if step >= 3 {
                                assert_eq!(
                                    ctx.output(|out| out.cursor_icon),
                                    egui::CursorIcon::Grabbing
                                );
                            }
                            if step == 2 {
                                before = view.rect(canvas, [1600, 1000], 1.0).center();
                            }
                            if step == 4 {
                                assert_eq!(
                                    view.rect(canvas, [1600, 1000], 1.0).center() - before,
                                    end - start + egui::vec2(15.0, 10.0),
                                    "space={space}, held={}, cancelled={}",
                                    pan.held,
                                    pan.cancelled_drag
                                );
                            }
                        });
                    },
                );
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn space_pointer_motion_without_button_and_cancellation() {
        // Release, popup, typing focus, window focus, and native-dialog cancellation.
        for guard in 0..5 {
            for image in 0..3 {
                let ctx = egui::Context::default();
                let mut pan = SpacePan::default();
                let mut view = View::default();
                if image == 2 {
                    view.actual_size();
                }
                let start = egui::pos2(100.0, 100.0);
                let delta = egui::vec2(40.0, -20.0);
                for step in 0..5 {
                    let mut events = match step {
                        0 => vec![egui::Event::PointerMoved(start)],
                        1 => vec![key_event(Key::Space, false)],
                        2 => vec![egui::Event::PointerMoved(start + delta)],
                        3 if guard == 0 => vec![egui::Event::Key {
                            key: Key::Space,
                            physical_key: None,
                            pressed: false,
                            repeat: false,
                            modifiers: Modifiers::NONE,
                        }],
                        4 => vec![egui::Event::PointerMoved(start + delta * 2.0)],
                        _ => vec![],
                    };
                    if step == 3 {
                        events.push(egui::Event::PointerMoved(start + delta * 1.5));
                    }
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(400.0, 300.0),
                            )),
                            focused: !(guard == 3 && step >= 3),
                            events,
                            ..Default::default()
                        },
                        |ui| {
                            if step == 3 {
                                match guard {
                                    1 => egui::Popup::open_id(&ctx, egui::Id::new("menu")),
                                    2 => ctx.memory_mut(|memory| {
                                        memory.request_focus(egui::Id::new("text"));
                                    }),
                                    4 => pan.update_key(false, false, false, true),
                                    _ => {}
                                }
                            }
                            let enabled = shortcuts_enabled(&ctx);
                            ctx.input(|input| {
                                assert!(!input.pointer.any_down());
                                pan.update_key(
                                    input.events.iter().any(|event| {
                                        matches!(
                                            event,
                                            egui::Event::Key {
                                                key: Key::Space,
                                                pressed: true,
                                                repeat: false,
                                                ..
                                            }
                                        )
                                    }),
                                    input.key_down(Key::Space),
                                    enabled,
                                    input.pointer.primary_down(),
                                );
                            });
                            egui::CentralPanel::default().show(ui, |ui| {
                                let canvas = ui.available_rect_before_wrap();
                                let response = ui.allocate_rect(canvas, Sense::click_and_drag());
                                let before = view.rect(canvas, [1600, 1000], 1.0).center();
                                if image != 0 && enabled {
                                    let enlarged = view.enlarged(canvas, [1600, 1000], 1.0);
                                    let motion = pan.pan_delta(&response, enlarged);
                                    view.pan(motion, canvas, [1600, 1000], 1.0);
                                    if response.hovered()
                                        && let Some(cursor) =
                                            pan.cursor(enlarged, motion != egui::Vec2::ZERO)
                                    {
                                        ctx.set_cursor_icon(cursor);
                                    }
                                }
                                let after = view.rect(canvas, [1600, 1000], 1.0).center();
                                assert_eq!(
                                    after - before,
                                    if step == 2 && image == 2 {
                                        delta
                                    } else {
                                        egui::Vec2::ZERO
                                    },
                                    "guard={guard}, image={image}, step={step}"
                                );
                                if step >= 1 {
                                    assert_eq!(
                                        ctx.output(|out| out.cursor_icon),
                                        match (image, step) {
                                            (2, 1) => egui::CursorIcon::Grab,
                                            (2, 2) => egui::CursorIcon::Grabbing,
                                            _ => egui::CursorIcon::Default,
                                        }
                                    );
                                }
                            });
                        },
                    );
                    output.textures_delta.clear();
                }
            }
        }
    }

    #[test]
    fn fitted_or_missing_image_keeps_arrow_even_with_space_and_drag() {
        for has_image in [false, true] {
            for space in [false, true] {
                let mut pan = SpacePan::default();
                pan.update_key(space, space, true, true);
                let ctx = egui::Context::default();
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let canvas = ui.available_rect_before_wrap();
                        let mut view = View::default();
                        if has_image {
                            let enlarged = view.enlarged(canvas, [1600, 1000], 1.0);
                            let before = view.rect(canvas, [1600, 1000], 1.0);
                            if pan.pan_drag(true, enlarged) {
                                view.pan(egui::vec2(55.0, -10.0), canvas, [1600, 1000], 1.0);
                            }
                            assert_eq!(view.rect(canvas, [1600, 1000], 1.0), before);
                            if let Some(cursor) = pan.cursor(enlarged, true) {
                                ctx.set_cursor_icon(cursor);
                            }
                        }
                    });
                });
                assert_eq!(
                    output.platform_output.cursor_icon,
                    egui::CursorIcon::Default
                );
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn space_hold_repeats_release_and_cursor() {
        let mut pan = SpacePan::default();
        assert_eq!(pan.cursor(true, false), None);
        pan.update_key(true, true, true, false);
        assert_eq!(pan.cursor(true, false), Some(egui::CursorIcon::Grab));
        for _ in 0..5 {
            pan.update_key(false, true, true, false); // repeat is not a fresh press
            assert!(pan.active(true));
        }
        assert!(pan.pan_drag(true, true));
        assert_eq!(pan.cursor(true, true), Some(egui::CursorIcon::Grabbing));
        pan.update_key(false, false, true, false);
        assert!(!pan.held);
        assert_eq!(pan.cursor(true, false), None);
    }

    #[test]
    fn space_release_mid_drag_requires_mouse_release() {
        let mut pan = SpacePan::default();
        pan.update_key(true, true, true, true);
        assert!(pan.pan_drag(true, true));
        pan.update_key(false, false, true, true);
        assert!(!pan.pan_drag(true, true));
        assert_eq!(pan.cursor(true, true), None);
        pan.update_key(true, true, true, true);
        assert!(!pan.pan_drag(true, true)); // re-holding cannot revive the cancelled gesture
        assert_eq!(pan.cursor(true, false), None);
        pan.update_key(false, false, true, false);
        assert!(pan.pan_drag(true, true)); // next ordinary drag is unaffected
    }

    #[test]
    fn space_focus_guard_cancels_and_requires_fresh_press() {
        let mut pan = SpacePan::default();
        pan.update_key(true, true, false, false);
        assert!(!pan.active(true));
        pan.update_key(false, true, true, false);
        assert!(!pan.active(true));
        pan.update_key(true, true, true, true);
        assert!(pan.pan_drag(true, true));
        pan.update_key(false, true, false, true); // popup, typing focus, or window focus loss
        assert!(!pan.active(true));
        assert!(!pan.pan_drag(true, true));
        assert_eq!(pan.cursor(true, true), None);
        assert_eq!(pan.cursor(true, false), None);
        pan.update_key(false, true, true, true);
        assert!(!pan.active(true));
    }

    #[test]
    fn space_fitted_image_is_noop_and_hold_can_wait_for_enlargement() {
        let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0));
        let mut view = View::default();
        let mut pan = SpacePan::default();
        pan.update_key(true, true, true, false);
        assert!(!view.enlarged(canvas, [1600, 800], 1.0));
        assert_eq!(
            pan.cursor(view.enlarged(canvas, [1600, 800], 1.0), false),
            None
        );
        let before = view.rect(canvas, [1600, 800], 1.0);
        view.pan(egui::vec2(10.0, 20.0), canvas, [1600, 800], 1.0);
        assert_eq!(view.rect(canvas, [1600, 800], 1.0), before);
        view.actual_size();
        assert!(pan.active(view.enlarged(canvas, [1600, 800], 1.0)));
        assert!(pan.pan_drag(true, true));
        view.pan(egui::vec2(10.0, 20.0), canvas, [1600, 800], 1.0);
        assert_eq!(
            view.rect(canvas, [1600, 800], 1.0).center(),
            canvas.center() + egui::vec2(10.0, 20.0)
        );
    }

    fn key_event(key: Key, repeat: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn space_does_not_dispatch_or_consume_navigation_or_zoom() {
        for repeat in [false, true] {
            for other in [None, Some(Key::ArrowRight), Some(Key::Plus)] {
                let ctx = egui::Context::default();
                let mut events = vec![key_event(Key::Space, repeat)];
                if let Some(key) = other {
                    events.push(key_event(key, false));
                }
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let command = keyboard_command(ui.ctx());
                        match other {
                            None => assert!(command.is_none()),
                            Some(Key::ArrowRight) => {
                                assert!(matches!(command, Some(Command::Navigate(Direction::Next))))
                            }
                            Some(Key::Plus) => {
                                assert!(matches!(command, Some(Command::Zoom(1.25))))
                            }
                            _ => unreachable!(),
                        }
                        assert!(ui.ctx().input(|input| input.key_pressed(Key::Space)));
                    },
                );
                output.textures_delta.clear();
            }
        }
    }

    fn modified_key(key: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn close_shortcut_requests_root_viewport_close_for_both_platforms() {
        for modifiers in [
            Modifiers::MAC_CMD | Modifiers::COMMAND,
            Modifiers::CTRL | Modifiers::COMMAND,
        ] {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![modified_key(Key::W, modifiers)],
                    ..Default::default()
                },
                |ui| {
                    close_shortcut(ui.ctx());
                    assert!(!ui.ctx().input(|i| i.key_pressed(Key::W)));
                },
            );
            output.textures_delta.clear();
            assert!(
                output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .contains(&egui::ViewportCommand::Close)
            );
        }
    }

    #[test]
    fn close_shortcut_is_not_consumed_when_menu_text_or_dialog_owns_input() {
        for guard in 0..3 {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    // The synchronous native dialog owns the window's focus;
                    // Viewer::ui does not run while pick_file is active.
                    focused: guard != 2,
                    events: vec![modified_key(Key::W, Modifiers::COMMAND)],
                    ..Default::default()
                },
                |ui| {
                    match guard {
                        0 => egui::Popup::open_id(ui.ctx(), egui::Id::new("menu")),
                        1 => ui
                            .ctx()
                            .memory_mut(|m| m.request_focus(egui::Id::new("text"))),
                        _ => {}
                    }
                    close_shortcut(ui.ctx());
                    assert!(ui.ctx().input(|i| i.key_pressed(Key::W)));
                },
            );
            output.textures_delta.clear();
            assert!(
                !output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .contains(&egui::ViewportCommand::Close)
            );
        }
    }

    #[test]
    fn close_shortcut_leaves_existing_bindings_and_unrelated_keys_alone() {
        let cases = [
            (Key::O, Modifiers::COMMAND, Some(Command::Open)),
            (Key::Num0, Modifiers::NONE, Some(Command::Fit)),
            (Key::Num1, Modifiers::NONE, Some(Command::Fit)),
            (Key::Num2, Modifiers::NONE, Some(Command::ActualSize)),
            (Key::Plus, Modifiers::SHIFT, Some(Command::Zoom(1.25))),
            (Key::Minus, Modifiers::NONE, Some(Command::Zoom(0.8))),
            (
                Key::R,
                Modifiers::NONE,
                Some(Command::Transform(Operation::Clockwise)),
            ),
            (
                Key::R,
                Modifiers::SHIFT,
                Some(Command::Transform(Operation::Counterclockwise)),
            ),
            (
                Key::M,
                Modifiers::NONE,
                Some(Command::Transform(Operation::Mirror)),
            ),
            (
                Key::F,
                Modifiers::NONE,
                Some(Command::Transform(Operation::Flip)),
            ),
            (
                Key::ArrowLeft,
                Modifiers::NONE,
                Some(Command::Navigate(Direction::Previous)),
            ),
            (
                Key::ArrowRight,
                Modifiers::NONE,
                Some(Command::Navigate(Direction::Next)),
            ),
            (
                Key::Home,
                Modifiers::NONE,
                Some(Command::Navigate(Direction::First)),
            ),
            (
                Key::End,
                Modifiers::NONE,
                Some(Command::Navigate(Direction::Last)),
            ),
            (Key::Space, Modifiers::NONE, None),
            (Key::W, Modifiers::NONE, None),
            (Key::W, Modifiers::ALT, None),
        ];
        for (key, modifiers, expected) in cases {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![modified_key(key, modifiers)],
                    ..Default::default()
                },
                |ui| {
                    close_shortcut(ui.ctx());
                    let actual = keyboard_command(ui.ctx());
                    match (actual, expected) {
                        (None, None)
                        | (Some(Command::Open), Some(Command::Open))
                        | (Some(Command::Fit), Some(Command::Fit))
                        | (Some(Command::ActualSize), Some(Command::ActualSize)) => {}
                        (Some(Command::Zoom(a)), Some(Command::Zoom(b))) => assert_eq!(a, b),
                        (Some(Command::Transform(a)), Some(Command::Transform(b))) => {
                            assert_eq!(format!("{a:?}"), format!("{b:?}"));
                        }
                        (Some(Command::Navigate(a)), Some(Command::Navigate(b))) => {
                            assert_eq!(std::mem::discriminant(&a), std::mem::discriminant(&b));
                        }
                        _ => panic!("shortcut changed for {key:?}"),
                    }
                    if expected.is_none() {
                        assert!(ui.ctx().input(|i| i.key_pressed(key)));
                    }
                },
            );
            output.textures_delta.clear();
            assert!(
                !output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .contains(&egui::ViewportCommand::Close)
            );
        }
    }

    #[test]
    fn existing_keyboard_guard_blocks_popup_typing_and_unfocused_window() {
        for guard in 0..3 {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    focused: guard != 2,
                    ..Default::default()
                },
                |ui| {
                    match guard {
                        0 => egui::Popup::open_id(ui.ctx(), egui::Id::new("menu")),
                        1 => ui
                            .ctx()
                            .memory_mut(|memory| memory.request_focus(egui::Id::new("text"))),
                        _ => {}
                    }
                    assert!(!shortcuts_enabled(ui.ctx()));
                },
            );
            output.textures_delta.clear();
        }
    }

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
    fn partial_tile_staging_cannot_commit_or_reset_the_prior() {
        let keys = crate::tiles::coverage([150, 100], 0, 62, [0.0, 0.0, 1.0, 1.0]);
        let mut display = Display::new(Status::Empty);
        let mut view = View::default();
        display.finish(0, Ok("prior"), None, &mut view);
        view.actual_size();
        display.generation = 1;
        display.loading();
        for count in 0..keys.len() {
            let coverage = crate::tiles::ready(&keys, |key| keys[..count].contains(key));
            assert!(!display.finish_tiles(1, "partial", coverage, None, &mut view));
            assert_eq!(display.image, Some("prior"));
            assert!(matches!(display.status, Status::Loading));
            let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 100.0));
            assert_eq!(view.scale(canvas, [150, 100], 1.0), 1.0);
        }
        let coverage = crate::tiles::ready(&keys, |_| true);
        assert!(display.finish_tiles(1, "complete", coverage, None, &mut view));
        assert_eq!(display.image, Some("complete"));
    }

    #[test]
    fn submitted_coverage_waits_for_confirmation_and_error_retains_prior() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let keys = crate::tiles::coverage([150, 100], 0, 62, [0.0, 0.0, 1.0, 1.0]);
        let mut uploads = crate::render::Uploads::default();
        let done = Arc::new(AtomicBool::new(false));
        uploads.submitted(1, Arc::clone(&done));
        let mut display = Display::new(Status::Empty);
        let mut view = View::default();
        display.finish(0, Ok("prior"), None, &mut view);
        view.actual_size();
        display.generation = 1;
        display.loading();
        // All keys have textures/gutters, but submission has not completed.
        assert!(crate::tiles::ready(&keys, |_| true).is_some());
        for frame in [1, 2] {
            let coverage = uploads
                .coverage(frame, &keys, |_| done.load(Ordering::Acquire))
                .unwrap();
            assert!(!display.finish_tiles(1, "next", coverage, None, &mut view));
            assert_eq!(display.image, Some("prior"));
        }
        done.store(true, Ordering::Release);
        // Even an immediately completed submission must wait for a later frame.
        assert!(uploads.coverage(1, &keys, |_| true).unwrap().is_none());
        uploads.inject_failure();
        let error = uploads.coverage(3, &keys, |_| true).err().unwrap();
        assert!(error.to_string().contains("Display unavailable"));
        display.fail(error.to_string());
        assert_eq!(display.image, Some("prior"));
        let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 100.0));
        assert_eq!(view.scale(canvas, [150, 100], 1.0), 1.0);
        assert!(matches!(display.status, Status::Error(_)));
    }

    #[test]
    fn many_tile_uploads_are_bounded_per_frame_and_across_selections() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let keys = crate::tiles::coverage([5000, 3000], 0, 1022, [0.0, 0.0, 1.0, 1.0]);
        let mut uploads = crate::render::Uploads::default();
        let mut completed = Vec::new();
        for (i, key) in keys.iter().enumerate() {
            let frame = i as u64 + 1;
            assert!(uploads.available(frame));
            let done = Arc::new(AtomicBool::new(false));
            uploads.submitted(frame, Arc::clone(&done));
            assert!(!uploads.available(frame));
            assert!(!uploads.available(frame + 1)); // also blocks another selection
            completed.push(*key);
            assert!(
                uploads
                    .coverage(frame, &keys, |k| completed.contains(k))
                    .unwrap()
                    .is_none()
            );
            done.store(true, Ordering::Release);
            assert!(!uploads.available(frame)); // no second tile even if GPU was fast
            assert!(
                2 * crate::tiles::tile([5000, 3000], *key, 1022).bytes_at_depth(4)
                    <= crate::tiles::STAGING_BYTES
            );
        }
        let mut display = Display::new(Status::Loading);
        let mut view = View::default();
        let coverage = uploads
            .coverage(keys.len() as u64 + 1, &keys, |k| completed.contains(k))
            .unwrap();
        assert!(display.finish_tiles(0, "complete", coverage, None, &mut view));
        assert!(keys.len() > 1);
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
            context: egui::Context::default(),
            animation: None,
            svg_worker: None,
            pending_svg: None,
            pending_animation: false,
            uploads: crate::render::Uploads::new(None),
            staging_frame: 0,
            pending_image: None,
            last_paint: None,
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
            settings: crate::settings::Settings::default(),
            settings_store: crate::settings::Store::session_only(),
            sort: Sort::default(),
            view: View::default(),
            space_pan: SpacePan::default(),
            render_state: None,
            clipboard: None,
            action_error: None,
            action_notice: None,
            default_handler_result: None,
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
        let animation_path = dir.0.join("animation.gif");
        let mut encoder =
            image::codecs::gif::GifEncoder::new(std::fs::File::create(&animation_path).unwrap());
        encoder
            .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                1,
                1,
                image::Rgba([255; 4]),
            )))
            .unwrap();
        drop(encoder);
        let version = FileVersion::read(&animation_path).unwrap();
        let duration = std::time::Duration::from_millis(100);
        let mut playback =
            Playback::new(version.clone(), viewer.context.clone(), duration).unwrap();
        playback.clock.toggle(std::time::Duration::ZERO);
        assert!(playback.clock.paused);
        viewer.animation = Some(playback);
        viewer.open(animation_path);
        assert!(
            viewer.animation.is_none(),
            "new selection cancels paused playback"
        );
        assert!(!viewer.pending_animation);
        // The same construction seam used on successful admission never inherits pause.
        let mut restarted = Playback::new(version, viewer.context.clone(), duration).unwrap();
        assert!(!restarted.clock.paused);
        assert_eq!(restarted.displayed_index, 0);
        restarted.displayed_index = 2;
        restarted.waiting = true;
        restarted.set_rate(duration, crate::animation::Rate::Fps10);
        let deadline = restarted.clock.deadline;
        assert!(deadline >= std::time::Duration::from_millis(100));
        assert!(deadline <= restarted.origin.elapsed() + std::time::Duration::from_millis(100));
        assert_eq!(
            restarted.displayed_index, 2,
            "live change does not reset position"
        );
        assert!(!restarted.waiting);
        assert!(restarted.resync);
        restarted.set_rate(duration, crate::animation::Rate::Off);
        assert!(!restarted.clock.due(std::time::Duration::MAX));
        assert_eq!(
            restarted.displayed_index, 2,
            "Off holds current composed frame"
        );
    }
}
