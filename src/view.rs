use eframe::egui::{Rect, Vec2};

pub fn fitted_rect(canvas: Rect, image: [usize; 2], pixels_per_point: f32) -> Rect {
    let natural_size = Vec2::new(image[0] as f32, image[1] as f32) / pixels_per_point;
    let scale = (canvas.width() / natural_size.x)
        .min(canvas.height() / natural_size.y)
        .clamp(0.0, 1.0);
    Rect::from_center_size(canvas.center(), natural_size * scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Pos2;

    #[test]
    fn fit_centers_without_enlargement_in_physical_pixels() {
        let canvas = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(400.0, 300.0));
        let small = fitted_rect(canvas, [100, 50], 2.0);
        assert_eq!(small.size(), Vec2::new(50.0, 25.0));
        assert_eq!(small.center(), canvas.center());
        let large = fitted_rect(canvas, [1600, 800], 2.0);
        assert_eq!(large.size(), Vec2::new(400.0, 200.0));
        assert_eq!(large.center(), canvas.center());
    }
}
