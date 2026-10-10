use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread,
};

use eframe::egui;

use crate::{
    decode::{self, LoadError, Pixels},
    folder::{Direction, Folder, Sort},
};

pub struct Request {
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
    shared.context.request_repaint();
    true
}

fn run(shared: Arc<Shared>) {
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
            let image = decode::load(&path, request.texture_limit);
            publish(&shared, request.generation, |result| {
                result.image = Some(image)
            });
        }
    }
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
