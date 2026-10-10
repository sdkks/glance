use crate::transform::{Operation, Transform};
use eframe::egui::{Pos2, Rect, Vec2};

pub const MIN_SCALE: f32 = 0.01;
pub const MAX_SCALE: f32 = 500.0;

#[derive(Clone, Default)]
pub struct View {
    // None follows fit dynamically on viewport/DPI changes; Some is physical pixel scale.
    scale: Option<f32>,
    pan: Vec2,
    pub transform: Transform,
}

impl View {
    pub fn fit(&mut self) {
        self.scale = None;
        self.pan = Vec2::ZERO;
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn transform(&mut self, operation: Operation) {
        self.transform.apply(operation);
    }

    pub fn actual_size(&mut self) {
        self.scale = Some(1.0);
        self.pan = Vec2::ZERO;
    }

    pub fn scale(&self, canvas: Rect, image: [usize; 2], ppp: f32) -> f32 {
        self.scale.unwrap_or_else(|| fit_scale(canvas, image, ppp))
    }

    pub fn rect(&self, canvas: Rect, image: [usize; 2], ppp: f32) -> Rect {
        Rect::from_center_size(
            canvas.center() + self.pan,
            source_size(image) * self.scale(canvas, image, ppp) / ppp,
        )
    }

    pub fn zoom(&mut self, factor: f32, pointer: Pos2, canvas: Rect, image: [usize; 2], ppp: f32) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let old = self.scale(canvas, image, ppp);
        let new = (old * factor).clamp(MIN_SCALE, MAX_SCALE);
        if old > 0.0 {
            self.pan =
                pointer - canvas.center() - (pointer - canvas.center() - self.pan) * (new / old);
        }
        self.scale = Some(new);
    }

    pub fn pan(&mut self, delta: Vec2, canvas: Rect, image: [usize; 2], ppp: f32) {
        let size = self.rect(canvas, image, ppp).size();
        if size.x > canvas.width() || size.y > canvas.height() {
            self.pan += delta;
        }
    }
}

fn source_size(image: [usize; 2]) -> Vec2 {
    Vec2::new(image[0] as f32, image[1] as f32)
}

fn fit_scale(canvas: Rect, image: [usize; 2], ppp: f32) -> f32 {
    let size = source_size(image) / ppp;
    (canvas.width() / size.x)
        .min(canvas.height() / size.y)
        .clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn canvas() -> Rect {
        Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(400.0, 300.0))
    }

    #[test]
    fn fit_preserves_orientation_reset_restores_base_and_framing() {
        let mut view = View::default();
        view.transform(Operation::Clockwise);
        view.actual_size();
        view.pan(Vec2::new(20.0, 30.0), canvas(), [1600, 800], 1.0);
        let orientation = view.transform;
        view.fit();
        assert_eq!(view.transform, orientation);
        assert_eq!(view.pan, Vec2::ZERO);
        view.actual_size();
        view.reset();
        assert_eq!(view.transform, Transform::default());
        assert_eq!(view.scale, None);
        assert_eq!(view.pan, Vec2::ZERO);
    }

    #[test]
    fn transform_preserves_zoom_pan_and_transformed_pointer_anchor() {
        let mut view = View::default();
        view.actual_size();
        view.pan(Vec2::new(20.0, 30.0), canvas(), [1600, 800], 1.0);
        view.transform(Operation::Clockwise);
        let size = view.transform.size([1600, 800]);
        assert_eq!(size, [800, 1600]);
        assert_eq!(view.scale(canvas(), size, 1.0), 1.0);
        assert_eq!(view.pan, Vec2::new(20.0, 30.0));
        let pointer = canvas().center() + Vec2::new(13.0, 27.0);
        let before = view.rect(canvas(), size, 1.0);
        let uv = (pointer - before.min) / before.size();
        view.zoom(2.0, pointer, canvas(), size, 1.0);
        let after = view.rect(canvas(), size, 1.0);
        assert!(((pointer - after.min) / after.size() - uv).length() < 0.0001);
    }

    #[test]
    fn fit_and_actual_size_are_physical_and_centered() {
        let canvas = canvas();
        let mut view = View::default();
        assert_eq!(
            view.rect(canvas, [100, 50], 2.0).size(),
            Vec2::new(50.0, 25.0)
        );
        assert_eq!(
            view.rect(canvas, [1600, 800], 2.0).size(),
            Vec2::new(400.0, 200.0)
        );
        view.actual_size();
        assert_eq!(
            view.rect(canvas, [1600, 800], 2.0).size(),
            Vec2::new(800.0, 400.0)
        );
        assert_eq!(
            view.rect(canvas, [1600, 800], 2.0).center(),
            canvas.center()
        );
        view.fit();
        assert_eq!(view.scale(canvas, [1600, 800], 2.0), 0.5);
    }

    #[test]
    fn pointer_anchor_and_clamps() {
        let canvas = canvas();
        let pointer = canvas.center() + Vec2::new(73.0, -21.0);
        let mut view = View::default();
        let before = view.rect(canvas, [1600, 800], 2.0);
        let uv = (pointer - before.min) / before.size();
        view.zoom(2.0, pointer, canvas, [1600, 800], 2.0);
        let after = view.rect(canvas, [1600, 800], 2.0);
        assert!(((pointer - after.min) / after.size() - uv).length() < 0.0001);
        view.zoom(f32::MAX, pointer, canvas, [1600, 800], 2.0);
        assert_eq!(view.scale(canvas, [1600, 800], 2.0), MAX_SCALE);
        view.zoom(f32::MIN_POSITIVE, pointer, canvas, [1600, 800], 2.0);
        assert_eq!(view.scale(canvas, [1600, 800], 2.0), MIN_SCALE);
    }

    #[test]
    fn pan_only_enlarged_and_reset_clears_it() {
        let mut view = View::default();
        let canvas = canvas();
        view.pan(Vec2::new(20.0, 30.0), canvas, [1600, 800], 2.0);
        assert_eq!(view.pan, Vec2::ZERO);
        view.actual_size();
        view.pan(Vec2::new(20.0, 30.0), canvas, [1600, 800], 2.0);
        assert_eq!(view.pan, Vec2::new(20.0, 30.0));
        view.fit();
        assert_eq!(view.pan, Vec2::ZERO);
    }
}
