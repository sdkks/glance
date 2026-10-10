use super::{Document, LoadError, Pixels};
use std::sync::{Arc, Condvar, Mutex};

struct Target {
    revision: u64,
    document: Arc<Document>,
    scale: f32,
}

#[derive(Default)]
struct Mailbox {
    revision: u64,
    target: Option<Target>,
    pending: bool,
    completion: Option<(u64, Result<Pixels, LoadError>)>,
    closed: bool,
}

struct Shared {
    mailbox: Mutex<Mailbox>,
    wake: Condvar,
    context: eframe::egui::Context,
}

// One running rasterization, one replaceable request and one result. A new
// revision clears the result, and obsolete running work cannot publish.
pub struct Worker {
    shared: Arc<Shared>,
}

pub fn needs_raster(current: f32, needed: f32, exact: bool) -> bool {
    if exact {
        (current - needed).abs() > needed * 0.001
    } else {
        needed > current * 1.5 || current > needed * 2.5
    }
}

impl Worker {
    pub fn new(context: eframe::egui::Context) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            mailbox: Mutex::new(Mailbox::default()),
            wake: Condvar::new(),
            context,
        });
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("glance-svg".into())
            .spawn(move || run(worker))?;
        Ok(Self { shared })
    }

    pub fn update(&self, pixels: &Pixels, needed: f32, exact: bool) {
        let Some(document) = &pixels.svg else { return };
        if !needed.is_finite() || needed <= 0.0 {
            return;
        }
        let mut mailbox = self.shared.mailbox.lock().expect("SVG mailbox poisoned");
        let same = mailbox
            .target
            .as_ref()
            .filter(|t| Arc::ptr_eq(&t.document, document));
        let scale = same.map_or(pixels.svg_scale, |t| t.scale);
        if !needs_raster(scale, needed, exact) {
            return;
        }
        mailbox.revision = mailbox
            .revision
            .checked_add(1)
            .expect("SVG revision exhausted");
        mailbox.target = Some(Target {
            revision: mailbox.revision,
            document: Arc::clone(document),
            scale: needed,
        });
        mailbox.pending = true;
        mailbox.completion = None;
        self.shared.wake.notify_one();
    }

    pub fn cancel(&self) {
        let mut mailbox = self.shared.mailbox.lock().expect("SVG mailbox poisoned");
        mailbox.revision = mailbox
            .revision
            .checked_add(1)
            .expect("SVG revision exhausted");
        mailbox.target = None;
        mailbox.pending = false;
        mailbox.completion = None;
    }

    pub fn current(&self, revision: u64, document: &Arc<Document>) -> bool {
        let mailbox = self.shared.mailbox.lock().expect("SVG mailbox poisoned");
        mailbox
            .target
            .as_ref()
            .is_some_and(|t| t.revision == revision && Arc::ptr_eq(&t.document, document))
    }

    pub fn take(&self) -> Option<(u64, Result<Pixels, LoadError>)> {
        self.shared
            .mailbox
            .lock()
            .expect("SVG mailbox poisoned")
            .completion
            .take()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let mut mailbox = self.shared.mailbox.lock().expect("SVG mailbox poisoned");
        mailbox.closed = true;
        mailbox.target = None;
        mailbox.completion = None;
        self.shared.wake.notify_one();
        // Like ordinary synchronous codecs, in-flight work finishes off-thread.
    }
}

fn publish(shared: &Shared, revision: u64, result: Result<Pixels, LoadError>) -> bool {
    let mut mailbox = shared.mailbox.lock().expect("SVG mailbox poisoned");
    if mailbox.closed || mailbox.revision != revision {
        return false;
    }
    mailbox.completion = Some((revision, result));
    shared.wake.notify_all();
    shared.context.request_repaint();
    true
}

fn run(shared: Arc<Shared>) {
    loop {
        let (revision, document, scale) = {
            let mut mailbox = shared.mailbox.lock().expect("SVG mailbox poisoned");
            while !mailbox.pending && !mailbox.closed {
                mailbox = shared.wake.wait(mailbox).expect("SVG mailbox poisoned");
            }
            if mailbox.closed {
                return;
            }
            mailbox.pending = false;
            let target = mailbox.target.as_ref().expect("pending SVG target");
            (target.revision, Arc::clone(&target.document), target.scale)
        };
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| document.rasterize(scale)))
                .unwrap_or_else(|_| Err(LoadError::Malformed("SVG: renderer panicked".into())));
        publish(&shared, revision, result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(worker: &Worker) -> Pixels {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut mailbox = worker.shared.mailbox.lock().unwrap();
        while mailbox.completion.is_none() {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(!remaining.is_zero(), "SVG worker failed to complete");
            mailbox = worker
                .shared
                .wake
                .wait_timeout(mailbox, remaining)
                .unwrap()
                .0;
        }
        mailbox.completion.take().unwrap().1.unwrap()
    }

    #[test]
    fn real_worker_fit_then_zoom_feeds_levels_and_intrinsic_copy() {
        let pixels = super::super::decode(b"<svg xmlns='http://www.w3.org/2000/svg' width='20' height='10'><rect width='10.5' height='10' fill='red'/></svg>", false).unwrap();
        let worker = Worker::new(eframe::egui::Context::default()).unwrap();
        worker.update(&pixels, 0.5, true);
        let fitted = wait(&worker);
        assert_eq!(fitted.size, [10, 5]);
        assert_eq!(fitted.levels[0].size, fitted.size);
        worker.update(&fitted, 2.0, false);
        let zoomed = wait(&worker);
        assert_eq!(zoomed.size, [40, 20]);
        assert_eq!(zoomed.rgba[20 * 4 + 3], 255);
        assert_eq!(zoomed.rgba[21 * 4 + 3], 0);
        assert!(Arc::ptr_eq(
            zoomed.svg.as_ref().unwrap(),
            pixels.svg.as_ref().unwrap()
        ));
        let copy = zoomed.svg.as_ref().unwrap().rasterize(1.0).unwrap();
        assert_eq!(copy.rgba, pixels.rgba);
        assert_eq!(copy.size, [20, 10]);
    }

    #[test]
    fn zoom_thresholds_fit_actual_and_stale_revisions() {
        let pixels = super::super::decode(
            b"<svg xmlns='http://www.w3.org/2000/svg' width='100' height='100'/>",
            false,
        )
        .unwrap();
        let worker = Worker {
            shared: Arc::new(Shared {
                mailbox: Mutex::new(Mailbox::default()),
                wake: Condvar::new(),
                context: eframe::egui::Context::default(),
            }),
        };
        assert!(!needs_raster(1.0, 1.5, false));
        assert!(needs_raster(1.0, 1.51, false));
        assert!(!needs_raster(2.5, 1.0, false));
        assert!(needs_raster(2.51, 1.0, false));
        let canvas = eframe::egui::Rect::from_min_size(
            eframe::egui::Pos2::ZERO,
            eframe::egui::vec2(25.0, 25.0),
        );
        let mut view = crate::view::View::default();
        worker.update(
            &pixels,
            view.scale(canvas, pixels.size, 2.0),
            view.exact_svg_scale(),
        );
        let first = worker.shared.mailbox.lock().unwrap().revision;
        assert_eq!(
            worker
                .shared
                .mailbox
                .lock()
                .unwrap()
                .target
                .as_ref()
                .unwrap()
                .scale,
            0.5
        );
        view.actual_size();
        worker.update(
            &pixels,
            view.scale(canvas, pixels.size, 2.0),
            view.exact_svg_scale(),
        );
        let second = worker.shared.mailbox.lock().unwrap().revision;
        assert!(second > first);
        assert!(!publish(&worker.shared, first, Ok(Pixels::default())));
        assert!(!worker.current(first, pixels.svg.as_ref().unwrap()));
        assert!(worker.current(second, pixels.svg.as_ref().unwrap()));
        let other = super::super::decode(
            b"<svg xmlns='http://www.w3.org/2000/svg' width='100' height='100'/>",
            false,
        )
        .unwrap();
        assert!(!worker.current(second, other.svg.as_ref().unwrap()));
        assert!(publish(&worker.shared, second, Ok(Pixels::default())));
        view.zoom(1.5, canvas.center(), canvas, pixels.size, 2.0);
        worker.update(
            &pixels,
            view.scale(canvas, pixels.size, 2.0),
            view.exact_svg_scale(),
        );
        assert!(worker.current(second, pixels.svg.as_ref().unwrap()));
        view.zoom(1.01, canvas.center(), canvas, pixels.size, 2.0);
        worker.update(
            &pixels,
            view.scale(canvas, pixels.size, 2.0),
            view.exact_svg_scale(),
        );
        assert!(!worker.current(second, pixels.svg.as_ref().unwrap()));
        assert!(worker.take().is_none());
        worker.cancel();
        assert!(worker.take().is_none());
        assert!(!publish(&worker.shared, second, Ok(Pixels::default())));
    }
}
