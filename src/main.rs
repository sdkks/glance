mod app;
mod cache;
mod decode;
mod desktop;
mod folder;
mod levels;
mod loader;
mod render;
mod tiles;
mod transform;
mod view;

use eframe::egui;

fn main() -> eframe::Result {
    let initial_path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([280.0, 180.0])
            .with_icon(egui::IconData::default()),
        ..Default::default()
    };
    eframe::run_native(
        "glance",
        options,
        Box::new(move |cc| Ok(Box::new(app::Viewer::new(cc, initial_path)))),
    )
}
