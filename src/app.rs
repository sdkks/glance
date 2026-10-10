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
            uploads: crate::render::Uploads::new(cc.wgpu_render_state.as_ref()),
            staging_frame: 0,
            pending_image: None,
            last_paint: None,
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
        self.pending_image = None;
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
        self.staging_frame = self.staging_frame.wrapping_add(1);
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).fill(Color32::from_gray(18)))
            .show(ui, |ui| {
                let canvas = ui.available_rect_before_wrap();
                let response = ui.allocate_rect(canvas, Sense::click_and_drag());
                response.context_menu(|ui| {
                    self.menus(ui, &mut command);
                });
                // Stage against the framing that will actually be committed. The
                // previous image/view remain intact until every visible tile is ready.
                if let Some((generation, version, image)) = self.pending_image.take()
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
                        Err(error) => self.display.fail(error),
                    }
                }
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
