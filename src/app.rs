use std::{path::PathBuf, sync::Arc};

use eframe::egui::{self, Color32, Key, Modifiers, Sense};

use crate::{
    folder::{Direction, Folder, Sort},
    loader::{Completion, Loader, Request},
    render::Image,
    view::View,
};

enum Display {
    Empty,
    Loading,
    Ready(Arc<Image>),
    Error(String),
}

#[derive(Clone, Copy)]
enum Command {
    Open,
    Navigate(Direction),
    Sort(Sort),
    Fit,
    ActualSize,
    Zoom(f32),
}

pub struct Viewer {
    loader: Option<Loader>,
    folder: Option<Folder>,
    selected: Option<PathBuf>,
    generation: u64,
    texture_limit: u32,
    display: Display,
    last_title: String,
    sort: Sort,
    view: View,
    render_state: Option<eframe::egui_wgpu::RenderState>,
}

impl Viewer {
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let (loader, display) = match Loader::new(cc.egui_ctx.clone()) {
            Ok(loader) => (Some(loader), Display::Empty),
            Err(error) => (
                None,
                Display::Error(format!("Could not start image loader: {error}")),
            ),
        };
        let mut viewer = Self {
            loader,
            folder: None,
            selected: None,
            generation: 0,
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
        };
        if let Some(path) = initial {
            viewer.open(path);
        }
        viewer
    }

    fn open(&mut self, path: PathBuf) {
        self.folder = None;
        self.selected = Some(path.clone());
        self.display = Display::Loading;
        self.view.fit();
        self.request(path, None);
    }

    fn request(&mut self, path: PathBuf, refresh: Option<(Folder, Option<Direction>)>) {
        let Some(generation) = self.generation.checked_add(1) else {
            self.display = Display::Error("Selection counter exhausted; restart glance.".into());
            return;
        };
        self.generation = generation;
        if let Some(loader) = &self.loader {
            loader.request(Request {
                generation,
                path,
                scan_folder: refresh.is_none(),
                texture_limit: self.texture_limit,
                sort: self.sort,
                refresh,
                load_unchanged: matches!(self.display, Display::Loading),
            });
        } else {
            self.display = Display::Error("Image loader unavailable; restart glance.".into());
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
                self.view.fit();
                self.display = Display::Loading;
            }
            self.request(path, Some((snapshot, direction)));
        }
    }

    fn admit(&mut self, result: Completion) {
        if result.generation != self.generation {
            return;
        }
        if let Some(folder) = result.folder {
            match folder {
                Ok(folder) => {
                    let selected = folder.selected_path().cloned();
                    if selected != self.selected {
                        self.view.fit();
                        self.display = if selected.is_some() {
                            Display::Loading
                        } else {
                            Display::Empty
                        };
                    }
                    self.selected = selected;
                    self.folder = Some(folder);
                }
                Err(error) => self.display = Display::Error(error.to_string()),
            }
        }
        if let Some(image) = result.image {
            self.display = match image {
                Ok(pixels) => {
                    let size = pixels.size;
                    match self
                        .render_state
                        .as_ref()
                        .ok_or_else(|| "GPU renderer unavailable".to_owned())
                        .and_then(|state| {
                            Image::upload(state, pixels).map_err(|error| error.to_string())
                        }) {
                        Ok(image) => {
                            eprintln!(
                                "Displayed {} ({} × {})",
                                self.selected
                                    .as_deref()
                                    .unwrap_or(std::path::Path::new(""))
                                    .display(),
                                size[0],
                                size[1]
                            );
                            Display::Ready(image)
                        }
                        Err(error) => Display::Error(error),
                    }
                }
                Err(error) => Display::Error(error.to_string()),
            };
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
            let state = match self.display {
                Display::Loading => " — Loading",
                Display::Error(_) => " — Error",
                _ => "",
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
            ui.add_enabled_ui(matches!(self.display, Display::Ready(_)), |ui| {
                for (label, action) in [
                    ("Fit  0 / 1", Command::Fit),
                    ("Actual Size (100%)  2", Command::ActualSize),
                    ("Zoom In  +", Command::Zoom(1.25)),
                    ("Zoom Out  −", Command::Zoom(0.8)),
                ] {
                    menu_command(ui, label, action, command);
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
        self.update_title(ctx);
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
            for (key, action) in [
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
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).fill(Color32::from_gray(18)))
            .show(ui, |ui| {
                let canvas = ui.available_rect_before_wrap();
                let response = ui.allocate_rect(canvas, Sense::click_and_drag());
                response.context_menu(|ui| {
                    self.menus(ui, &mut command);
                });
                if let Display::Ready(image) = &self.display {
                    let size = image.size;
                    let ppp = ctx.pixels_per_point();
                    if let Some(action) = command {
                        match action {
                            Command::Fit => self.view.fit(),
                            Command::ActualSize => self.view.actual_size(),
                            Command::Zoom(factor) => {
                                self.view.zoom(factor, canvas.center(), canvas, size, ppp)
                            }
                            _ => {}
                        }
                    }
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
                        ));
                    }
                } else {
                    let path = self.selected.as_deref().unwrap_or(std::path::Path::new(""));
                    let message = match &self.display {
                        Display::Empty if self.folder.is_some() => {
                            "No JPEG or PNG images in this folder.".to_owned()
                        }
                        Display::Empty => {
                            "Open an image, or drop a file or folder here.".to_owned()
                        }
                        Display::Loading => format!("Loading {}…", path.display()),
                        Display::Error(error) => format!("{}\n\n{error}", path.display()),
                        Display::Ready(_) => unreachable!(),
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
            Some(Command::Open) => self.dialog(frame),
            Some(Command::Navigate(direction)) => self.refresh(Some(direction)),
            Some(Command::Sort(sort)) => {
                self.sort = sort;
                self.refresh(None);
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
    fn stale_results_are_rejected_and_error_keeps_folder() {
        let dir = TempDir::new();
        let path = dir.0.join("latest.png");
        let mut viewer = Viewer {
            loader: None,
            folder: None,
            selected: Some(path.clone()),
            generation: 3,
            texture_limit: 8192,
            display: Display::Loading,
            last_title: String::new(),
            sort: Sort::default(),
            view: View::default(),
            render_state: None,
        };
        viewer.admit(Completion {
            generation: 2,
            image: Some(Err(LoadError::Unsupported)),
            ..Default::default()
        });
        assert!(matches!(viewer.display, Display::Loading));
        viewer.admit(Completion {
            generation: 3,
            folder: Some(Ok(Folder::open(path.clone()).unwrap())),
            image: Some(Err(LoadError::Unsupported)),
        });
        assert!(matches!(viewer.display, Display::Error(_)));
        assert_eq!(viewer.folder.as_ref().unwrap().selected_path(), Some(&path));
    }
}
