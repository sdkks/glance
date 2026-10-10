use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread,
};

use eframe::egui;

use crate::{
    cache::Ready,
    decode::{self, LoadError, Pixels},
    folder::{Direction, FileVersion, Folder, Sort},
};

pub struct Request {
    pub cached: Vec<(FileVersion, u64, Arc<Ready>)>,
    pub generation: u64,
    pub path: PathBuf,
    pub scan_folder: bool,
    pub texture_limit: u32,
    pub sort: Sort,
    pub refresh: Option<(Folder, Option<Direction>)>,
    pub load_unchanged: bool,
}

#[derive(Default)]
pub struct Completion {
    pub resident: Option<Arc<Ready>>,
    pub version: Option<FileVersion>,
    pub generation: u64,
    pub folder: Option<Result<Folder, LoadError>>,
    pub image: Option<Result<Pixels, LoadError>>,
}

#[derive(Default)]
struct Mailbox {
    latest: u64,
    pending: Option<Request>,
    completion: Completion,
    closed: bool,
}

struct Shared {
    mailbox: Mutex<Mailbox>,
    wake: Condvar,
    context: egui::Context,
}

pub struct Loader {
    shared: Arc<Shared>,
}

impl Loader {
    pub fn new(context: egui::Context) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            mailbox: Mutex::new(Mailbox::default()),
            wake: Condvar::new(),
            context,
        });
        let worker = Arc::clone(&shared);
        thread::Builder::new()
            .name("glance-loader".into())
            .spawn(move || run(worker))?;
        Ok(Self { shared })
    }

    pub fn request(&self, request: Request) {
        let mut mailbox = self.shared.mailbox.lock().expect("mailbox lock poisoned");
        mailbox.latest = request.generation;
        mailbox.completion = Completion {
            generation: request.generation,
            ..Default::default()
        };
        // One replaceable pending selection; never queue one allocation/thread per keypress.
        mailbox.pending = Some(request);
        self.shared.wake.notify_one();
    }

    pub fn cancel(&self, generation: u64) {
        let mut mailbox = self.shared.mailbox.lock().expect("mailbox lock poisoned");
        mailbox.latest = generation;
        mailbox.pending = None;
        mailbox.completion = Completion::default();
    }

    pub fn take_completion(&self) -> Completion {
        let mut mailbox = self.shared.mailbox.lock().expect("mailbox lock poisoned");
        std::mem::take(&mut mailbox.completion)
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        let mut mailbox = self.shared.mailbox.lock().expect("mailbox lock poisoned");
        mailbox.closed = true;
        mailbox.pending = None;
        mailbox.completion = Completion::default();
        self.shared.wake.notify_one();
        // A synchronous codec may still be running. Do not join it on the UI thread.
    }
}

fn publish(shared: &Shared, generation: u64, update: impl FnOnce(&mut Completion)) -> bool {
    let mut mailbox = shared.mailbox.lock().expect("mailbox lock poisoned");
    if mailbox.closed || mailbox.latest != generation {
        return false;
    }
    mailbox.completion.generation = generation;
    update(&mut mailbox.completion);
    shared.wake.notify_all();
    shared.context.request_repaint();
    true
}

fn run(shared: Arc<Shared>) {
    run_with(shared, decode::load);
}

fn run_with(
    shared: Arc<Shared>,
    load: impl Fn(&std::path::Path, u32) -> Result<Pixels, LoadError>,
) {
    loop {
        let request = {
            let mut mailbox = shared.mailbox.lock().expect("mailbox lock poisoned");
            while mailbox.pending.is_none() && !mailbox.closed {
                mailbox = shared.wake.wait(mailbox).expect("mailbox lock poisoned");
            }
            if mailbox.closed {
                return;
            }
            mailbox
                .pending
                .take()
                .expect("pending request checked above")
        };
        let path = if let Some((mut folder, direction)) = request.refresh {
            match folder.refresh(direction, request.sort) {
                Ok(changed) => {
                    let path = (changed || request.load_unchanged)
                        .then(|| folder.selected_path().cloned())
                        .flatten();
                    if !publish(&shared, request.generation, |result| {
                        result.folder = Some(Ok(folder))
                    }) {
                        continue;
                    }
                    path
                }
                Err(error) => {
                    publish(&shared, request.generation, |result| {
                        result.folder = Some(Err(error.into()))
                    });
                    continue;
                }
            }
        } else if request.scan_folder {
            match Folder::open_sorted(request.path.clone(), request.sort) {
                Ok(folder) => {
                    let path = folder.selected_path().cloned();
                    if !publish(&shared, request.generation, |result| {
                        result.folder = Some(Ok(folder))
                    }) {
                        continue;
                    }
                    path
                }
                Err(error) => {
                    publish(&shared, request.generation, |result| {
                        result.folder = Some(Err(error.into()))
                    });
                    continue;
                }
            }
        } else {
            Some(request.path)
        };
        if let Some(path) = path {
            let current = {
                let mailbox = shared.mailbox.lock().expect("mailbox lock poisoned");
                !mailbox.closed && mailbox.latest == request.generation
            };
            if !current {
                continue;
            }
            let version = FileVersion::read(&path).ok();
            if let Some(resident) = find_cached(request.cached, version.as_ref()) {
                publish(&shared, request.generation, |result| {
                    result.version = version;
                    result.resident = Some(resident);
                });
                continue;
            }
            let image = load(&path, request.texture_limit);
            let after = FileVersion::read(&path).ok();
            let image = if version != after {
                Err(LoadError::Malformed(
                    "File changed during decoding; retry opening it.".into(),
                ))
            } else {
                image
            };
            publish(&shared, request.generation, |result| {
                result.version = version;
                result.image = Some(image)
            });
        }
    }
}

pub(crate) fn find_cached<T>(
    entries: Vec<(FileVersion, u64, Arc<T>)>,
    version: Option<&FileVersion>,
) -> Option<Arc<T>> {
    entries
        .into_iter()
        .find(|(v, _, _)| Some(v) == version)
        .map(|(_, _, value)| value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> Shared {
        Shared {
            mailbox: Mutex::new(Mailbox::default()),
            wake: Condvar::new(),
            context: egui::Context::default(),
        }
    }

    fn request_for(generation: u64, path: PathBuf) -> Request {
        Request {
            cached: Vec::new(),
            generation,
            path,
            scan_folder: false,
            texture_limit: 8192,
            sort: Sort::default(),
            refresh: None,
            load_unchanged: true,
        }
    }

    fn wait_image(shared: &Shared) -> Completion {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut mailbox = shared.mailbox.lock().unwrap();
        while mailbox.completion.image.is_none() {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(!left.is_zero(), "worker failed to complete");
            mailbox = shared.wake.wait_timeout(mailbox, left).unwrap().0;
        }
        std::mem::take(&mut mailbox.completion)
    }

    #[test]
    fn blocked_prefetch_cannot_queue_block_or_displace_foreground() {
        let foreground = Loader {
            shared: Arc::new(shared()),
        };
        let prefetch = Loader {
            shared: Arc::new(shared()),
        };
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let background_shared = Arc::clone(&prefetch.shared);
        let background = thread::spawn(move || {
            run_with(background_shared, |_, _| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Err(LoadError::Unsupported)
            })
        });
        prefetch.request(request_for(1, "prefetch.png".into()));
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        foreground.request(request_for(2, "foreground.png".into()));
        prefetch.request(request_for(3, "another-prefetch.png".into()));
        assert_eq!(
            foreground
                .shared
                .mailbox
                .lock()
                .unwrap()
                .pending
                .as_ref()
                .unwrap()
                .generation,
            2
        );
        let front_shared = Arc::clone(&foreground.shared);
        let front =
            thread::spawn(move || run_with(front_shared, |_, _| Err(LoadError::Unsupported)));
        assert_eq!(wait_image(&foreground.shared).generation, 2);
        prefetch.cancel(4);
        release_tx.send(()).unwrap();
        drop(prefetch);
        drop(foreground);
        background.join().unwrap();
        front.join().unwrap();
    }

    #[test]
    fn navigating_faster_than_prefetch_rejects_late_pixels_and_pending_work() {
        let prefetch = Loader {
            shared: Arc::new(shared()),
        };
        prefetch.request(request_for(1, "old.png".into()));
        prefetch.cancel(2);
        assert!(!publish(&prefetch.shared, 1, |result| result.image =
            Some(Ok(Pixels {
                size: [1, 1],
                rgba: vec![0; 4]
            }))));
        assert!(prefetch.shared.mailbox.lock().unwrap().pending.is_none());
        assert!(prefetch.take_completion().image.is_none());
        prefetch.request(request_for(3, "new.png".into()));
        assert!(publish(&prefetch.shared, 3, |result| result.image =
            Some(Err(LoadError::Unsupported))));
        assert_eq!(prefetch.take_completion().generation, 3);
    }

    #[test]
    fn foreground_request_uses_prefetched_pixels_without_calling_decoder() {
        let dir = crate::folder::tests::TempDir::new();
        let path = dir.0.join("cached.png");
        std::fs::write(&path, b"not decodable: the cache must be used").unwrap();
        let version = FileVersion::read(&path).unwrap();
        let ready = Arc::new(Ready::Pixels(Arc::new(Pixels {
            size: [1, 1],
            rgba: vec![42; 4],
        })));
        let loader = Loader {
            shared: Arc::new(shared()),
        };
        let worker_shared = Arc::clone(&loader.shared);
        let worker = thread::spawn(move || {
            run_with(worker_shared, |_, _| panic!("cache hit must not decode"))
        });
        let mut request = request_for(9, path);
        request
            .cached
            .push((version.clone(), 1, Arc::clone(&ready)));
        loader.request(request);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut mailbox = loader.shared.mailbox.lock().unwrap();
        while mailbox.completion.resident.is_none() {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(!left.is_zero(), "cache completion missing");
            mailbox = loader.shared.wake.wait_timeout(mailbox, left).unwrap().0;
        }
        assert!(mailbox.completion.image.is_none());
        assert_eq!(mailbox.completion.generation, 9);
        assert_eq!(mailbox.completion.version, Some(version));
        assert!(Arc::ptr_eq(
            mailbox.completion.resident.as_ref().unwrap(),
            &ready
        ));
        drop(mailbox);
        drop(loader);
        worker.join().unwrap();
    }

    #[test]
    fn version_change_during_decode_is_not_admitted() {
        let dir = crate::folder::tests::TempDir::new();
        let path = dir.0.join("image.png");
        std::fs::write(&path, b"original").unwrap();
        let loader = Loader {
            shared: Arc::new(shared()),
        };
        let worker_shared = Arc::clone(&loader.shared);
        let worker = thread::spawn(move || {
            run_with(worker_shared, |path, _| {
                std::fs::write(path, b"changed file").unwrap();
                Ok(Pixels {
                    size: [1, 1],
                    rgba: vec![0; 4],
                })
            })
        });
        loader.request(request_for(1, path));
        assert!(matches!(
            wait_image(&loader.shared).image,
            Some(Err(LoadError::Malformed(_)))
        ));
        drop(loader);
        worker.join().unwrap();
    }

    #[test]
    fn stale_completions_cannot_replace_latest_selection() {
        let shared = shared();
        shared.mailbox.lock().unwrap().latest = 3;
        for generation in [1, 2] {
            assert!(!publish(&shared, generation, |result| result.image =
                Some(Err(LoadError::Unsupported))));
        }
        assert!(shared.mailbox.lock().unwrap().completion.image.is_none());
        assert!(publish(&shared, 3, |result| result.image =
            Some(Err(LoadError::Unsupported))));
        shared.mailbox.lock().unwrap().closed = true;
        assert!(!publish(&shared, 3, |_| panic!(
            "must not publish after close"
        )));
    }

    #[test]
    fn new_request_releases_unadmitted_pixels_immediately() {
        let loader = Loader {
            shared: Arc::new(shared()),
        };
        {
            let mut mailbox = loader.shared.mailbox.lock().unwrap();
            mailbox.latest = 1;
            mailbox.completion = Completion {
                generation: 1,
                image: Some(Ok(Pixels {
                    size: [1, 1],
                    rgba: vec![255; 4],
                })),
                ..Default::default()
            };
        }
        loader.request(Request {
            cached: Vec::new(),
            generation: 2,
            path: "next.png".into(),
            scan_folder: false,
            texture_limit: 8192,
            sort: Sort::default(),
            refresh: None,
            load_unchanged: true,
        });
        let mailbox = loader.shared.mailbox.lock().unwrap();
        assert_eq!(mailbox.completion.generation, 2);
        assert!(mailbox.completion.image.is_none());
        assert_eq!(mailbox.pending.as_ref().unwrap().generation, 2);
    }

    #[test]
    fn rapid_requests_keep_only_latest_pending_request() {
        let loader = Loader {
            shared: Arc::new(shared()),
        };
        for generation in 1..=3 {
            loader.request(Request {
                cached: Vec::new(),
                generation,
                path: PathBuf::from(format!("{generation}.png")),
                scan_folder: false,
                texture_limit: 8192,
                sort: Sort::default(),
                refresh: None,
                load_unchanged: true,
            });
        }
        let mailbox = loader.shared.mailbox.lock().unwrap();
        assert_eq!(mailbox.pending.as_ref().unwrap().generation, 3);
        assert_eq!(
            mailbox.pending.as_ref().unwrap().path,
            PathBuf::from("3.png")
        );
    }
}
