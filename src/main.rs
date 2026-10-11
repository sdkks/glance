mod animation;
mod app;
mod associations;
mod cache;
mod color;
mod decode;
mod default_handler;
mod desktop;
mod folder;
mod formats;
mod levels;
mod loader;
#[cfg(any(target_os = "macos", test))]
mod open_documents;
mod render;
mod settings;
mod tiles;
mod transform;
mod view;

use eframe::egui;

fn window_icon() -> egui::IconData {
    #[cfg(target_os = "macos")]
    let png = include_bytes!("../assets/icons/glance.iconset/icon_256x256.png");
    #[cfg(not(target_os = "macos"))]
    let png = include_bytes!("../assets/icons/hicolor/256x256/apps/glance.png");
    // The bundled PNG is validated at build/review time, not user input.
    eframe::icon_data::from_png_bytes(png).expect("bundled Glance icon must be a valid PNG")
}

fn main() -> eframe::Result {
    let initial_path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    // Packaging diagnostic: exercise the real decoder without a desktop session.
    if std::env::var_os("GLANCE_SMOKE_IMAGE").is_some_and(|v| v == "1") {
        let result = initial_path
            .as_deref()
            .ok_or_else(|| "smoke requires an image path".to_owned())
            .and_then(|path| decode::load(path, 16384).map_err(|error| error.to_string()));
        match result {
            Ok(pixels) => {
                println!("decoded {}x{}", pixels.size[0], pixels.size[1]);
                return Ok(());
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }
    #[cfg(target_os = "macos")]
    open_documents::register(
        objc2::MainThreadMarker::new().expect("the viewer must start on the main thread"),
    );
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([280.0, 180.0])
            .with_icon(window_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "glance",
        options,
        Box::new(move |cc| Ok(Box::new(app::Viewer::new(cc, initial_path)))),
    )
}

#[cfg(test)]
mod icon_tests {
    #[test]
    fn bundled_icons_decode_at_256_pixels() {
        for png in [
            include_bytes!("../assets/icons/glance.iconset/icon_256x256.png").as_slice(),
            include_bytes!("../assets/icons/hicolor/256x256/apps/glance.png").as_slice(),
        ] {
            let icon = eframe::icon_data::from_png_bytes(png).unwrap();
            assert_eq!((icon.width, icon.height), (256, 256));
            assert_eq!(icon.rgba.len(), 256 * 256 * 4);
            assert!(icon.rgba.chunks_exact(4).any(|pixel| pixel[3] != 0));
        }
        assert_eq!(super::window_icon().width, 256);
    }
}
