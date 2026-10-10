use std::path::PathBuf;

use eframe::egui::{self, Color32, Key, Modifiers, Rect, Sense, TextureHandle, TextureOptions};

use crate::{
    decode::LoadError,
    folder::{Direction, Folder},
    loader::{Completion, Loader, Request},
    view,
};

enum Display {
    Empty,
    Loading,
    Ready(TextureHandle),
    Error(String),
}

pub struct Viewer {
    loader: Option<Loader>,
    folder: Option<Folder>,
    selected: Option<PathBuf>,
    generation: u64,
    texture_limit: u32,
    display: Display,
    last_title: String,
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
        };
        if let Some(path) = initial {
            viewer.open(path);
        }
        viewer
    }

    fn open(&mut self, path: PathBuf) {
        self.folder = None;
        self.selected = Some(path.clone());
        self.request(path, true);
    }

    fn request(&mut self, path: PathBuf, scan_folder: bool) {
        let Some(generation) = self.generation.checked_add(1) else {
            self.display = Display::Error("Selection counter exhausted; restart glance.".into());
            return;
        };
        self.generation = generation;
        self.display = Display::Loading; // Drop the old texture: it must never masquerade as the new image.
        if let Some(loader) = &self.loader {
            loader.request(Request {
                generation,
                path,
                scan_folder,
                texture_limit: self.texture_limit,
            });
        } else {
            self.display = Display::Error("Image loader unavailable; restart glance.".into());
        }
    }

    fn navigate(&mut self, direction: Direction) {
        if let Some(folder) = &mut self.folder
            && folder.navigate(direction)
            && let Some(path) = folder.selected_path().cloned()
        {
            self.selected = Some(path.clone());
            self.request(path, false);
        }
    }

    fn admit(&mut self, result: Completion, ctx: &egui::Context) {
        if result.generation != self.generation {
            return;
        }
        if let Some(folder) = result.folder {
            match folder {
                Ok(folder) => {
                    self.selected = folder.selected_path().cloned();
                    if self.selected.is_none() {
                        self.display = Display::Empty;
                    }
                    self.folder = Some(folder);
                }
                Err(error) => self.display = Display::Error(error.to_string()),
            }
        }
        if let Some(image) = result.image {
            self.display = match image {
                Ok(pixels) => {
                    let max_side = ctx.input(|input| input.max_texture_side);
                    if pixels.size.iter().any(|&side| side > max_side) {
                        Display::Error(
                            LoadError::ResourceLimit(format!("display maximum side {max_side}"))
                                .to_string(),
                        )
                    } else {
                        let image =
                            egui::ColorImage::from_rgba_unmultiplied(pixels.size, &pixels.rgba);
                        let texture =
                            ctx.load_texture("current image", image, TextureOptions::LINEAR);
                        eprintln!(
                            "Displayed {} ({} × {})",
                            self.selected
                                .as_deref()
                                .unwrap_or(std::path::Path::new(""))
                                .display(),
                            pixels.size[0],
                            pixels.size[1]
                        );
                        Display::Ready(texture)
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
}

impl eframe::App for Viewer {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(loader) = &self.loader {
            self.admit(loader.take_completion(), ctx);
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
        let mut open = shortcuts_enabled
            && ctx.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::O));
        if shortcuts_enabled {
            for (key, direction) in [
                (Key::ArrowLeft, Direction::Previous),
                (Key::ArrowRight, Direction::Next),
                (Key::Home, Direction::First),
                (Key::End, Direction::Last),
            ] {
                if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, key)) {
                    self.navigate(direction);
                }
            }
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).fill(Color32::from_gray(18)))
            .show(ui, |ui| {
                let canvas = ui.available_rect_before_wrap();
                let response = ui.allocate_rect(canvas, Sense::click());
                response.context_menu(|ui| {
                    open |= ui.button("Open…  ⌘/Ctrl+O").clicked();
                    if open {
                        ui.close();
                    }
                });
                match &self.display {
                    Display::Ready(texture) => {
                        let rect =
                            view::fitted_rect(canvas, texture.size(), ctx.pixels_per_point());
                        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                        ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
                    }
                    display => {
                        let path = self.selected.as_deref().unwrap_or(std::path::Path::new(""));
                        let message = match display {
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
                        let layout = egui::Layout::centered_and_justified(egui::Direction::TopDown);
                        let builder = egui::UiBuilder::new().max_rect(canvas).layout(layout);
                        ui.scope_builder(builder, |ui| {
                            ui.vertical_centered(|ui| {
                                ui.label(message);
                                open |= ui.button("Open…").clicked();
                                if self.folder.is_some() {
                                    ui.label(
                                        "Left / Right: previous / next • Home / End: first / last",
                                    );
                                }
                            });
                        });
                    }
                }
            });
        if open {
            self.dialog(frame);
        }
        self.update_title(&ctx);
        // Worker completions and input events wake eframe. A settled image needs no timer.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_rejects_stale_results_and_keeps_navigation_after_error() {
        let ctx = egui::Context::default();
        let mut viewer = Viewer {
            loader: None,
            folder: None,
            selected: Some(PathBuf::from("latest.png")),
            generation: 3,
            texture_limit: 8192,
            display: Display::Loading,
            last_title: String::new(),
        };
        viewer.admit(
            Completion {
                generation: 2,
                image: Some(Err(LoadError::Unsupported)),
                ..Default::default()
            },
            &ctx,
        );
        assert!(matches!(viewer.display, Display::Loading));
        viewer.admit(
            Completion {
                generation: 3,
                folder: Some(Ok(Folder {
                    files: vec!["latest.png".into(), "next.png".into()],
                    selected: Some(0),
                })),
                image: Some(Err(LoadError::Unsupported)),
            },
            &ctx,
        );
        assert!(matches!(viewer.display, Display::Error(_)));
        viewer.navigate(Direction::Next);
        assert_eq!(viewer.selected, Some(PathBuf::from("next.png")));
        assert_eq!(viewer.generation, 4);
    }
}
