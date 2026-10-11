use std::path::{Path, PathBuf};

// Only local absolute paths are accepted, including the string fallback for
// descriptors that cannot be coerced to NSURL. Preserve URL authority checks.
fn document_path(file_url: Option<&str>, text: Option<&str>) -> Option<PathBuf> {
    file_url
        .and_then(crate::desktop::parse_reference)
        .or_else(|| text.and_then(crate::desktop::parse_reference))
}

#[derive(Default)]
struct DocumentQueue(Vec<PathBuf>);

impl DocumentQueue {
    fn push(&mut self, path: PathBuf) {
        self.0.push(path);
    }

    // Match the viewer's file-drop behavior: only the first document is opened.
    // Consume the entire batch so later frames do not reopen or cycle through it.
    fn drain(&mut self, selected: Option<&Path>) -> Option<PathBuf> {
        std::mem::take(&mut self.0)
            .into_iter()
            .next()
            .filter(|path| Some(path.as_path()) != selected)
    }
}

fn event_trace(descriptors: isize, queued: &[PathBuf]) -> String {
    format!(
        "GLANCE_TRACE_OPEN descriptors={descriptors} queued={} first={:?}",
        queued.len(),
        queued.first().map(|path| path.as_path())
    )
}

fn registration_trace(point: &str, registered: bool, pid: u32) -> String {
    let status = if registered {
        "registered"
    } else {
        "skipped already-done"
    };
    format!("GLANCE_TRACE_OPEN {point} aevt/odoc registration {status} pid={pid}")
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use eframe::egui;
    use objc2::{MainThreadOnly, define_class, msg_send, sel};
    use objc2_app_kit::{
        NSApplicationDidFinishLaunchingNotification, NSApplicationWillFinishLaunchingNotification,
    };
    use objc2_core_services::{
        kAEOpenDocuments, kCoreEventClass, keyDirectObject, typeAlias, typeBookmarkData, typeFSRef,
        typeFileURL,
    };
    use objc2_foundation::{
        MainThreadMarker, NSAppleEventDescriptor, NSAppleEventManager, NSNotification,
        NSNotificationCenter, NSObject,
    };
    use std::sync::{Mutex, Once, OnceLock};

    static DOCUMENTS: Mutex<DocumentQueue> = Mutex::new(DocumentQueue(Vec::new()));
    static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

    fn trace_enabled() -> bool {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| std::env::var_os("GLANCE_TRACE_OPEN").is_some_and(|v| v == "1"))
    }

    pub fn trace_open(path: &Path) {
        if trace_enabled() {
            eprintln!("GLANCE_TRACE_OPEN viewer opening queued document {path:?}");
        }
    }

    fn descriptor_path(descriptor: &NSAppleEventDescriptor) -> Option<PathBuf> {
        // Coercing arbitrary text to a file URL treats it as a legacy HFS path
        // (even an https URL becomes local). Only coerce native file descriptors;
        // text descriptors must pass our literal reference validation instead.
        let url = [typeAlias, typeBookmarkData, typeFSRef, typeFileURL]
            .contains(&descriptor.descriptorType())
            .then(|| descriptor.fileURLValue())
            .flatten()
            .and_then(|url| url.absoluteString());
        let text = descriptor.stringValue();
        document_path(
            url.as_ref().map(|s| s.to_string()).as_deref(),
            text.as_ref().map(|s| s.to_string()).as_deref(),
        )
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements; this class has no
        // Drop implementation and is allocated/registered on the main thread.
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        struct OpenDocumentsHandler;

        impl OpenDocumentsHandler {
            // SAFETY: NSAppleEventManager invokes this selector with two
            // NSAppleEventDescriptor objects and expects a void return.
            #[unsafe(method(handleOpenDocuments:withReplyEvent:))]
            fn handle_open_documents(
                &self,
                event: &NSAppleEventDescriptor,
                reply: &NSAppleEventDescriptor,
            ) {
                let trace = trace_enabled();
                if trace {
                    eprintln!("GLANCE_TRACE_OPEN handler invoked pid={}", std::process::id());
                }
                if let Some(list) = event.paramDescriptorForKeyword(keyDirectObject) {
                    let mut queue = DOCUMENTS.lock().unwrap_or_else(|e| e.into_inner());
                    let start = queue.0.len();
                    let descriptors = list.numberOfItems();
                    // AppleEvent list indices are one-based.
                    for index in 1..=descriptors {
                        if let Some(path) = list.descriptorAtIndex(index)
                            .and_then(|descriptor| descriptor_path(&descriptor))
                        {
                            queue.push(path);
                        }
                    }
                    if trace {
                        eprintln!("{}", event_trace(descriptors, &queue.0[start..]));
                    }
                } else if trace {
                    eprintln!("{}", event_trace(0, &[]));
                }
                reply.setParamDescriptor_forKeyword(
                    &NSAppleEventDescriptor::nullDescriptor(), keyDirectObject,
                );
                if let Some(ctx) = CONTEXT.get() {
                    ctx.request_repaint();
                }
            }
        }
    );

    define_class!(
        // SAFETY: NSObject has no subclassing requirements; callbacks are sent
        // by AppKit on the main thread and this class has no Drop implementation.
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        struct LaunchObserver;

        impl LaunchObserver {
            // SAFETY: NotificationCenter sends one NSNotification and expects void.
            #[unsafe(method(applicationWillFinishLaunching:))]
            fn application_will_finish_launching(&self, notification: &NSNotification) {
                // SAFETY: AppKit's notification-name constants are immutable.
                let point = if &*notification.name() == unsafe { NSApplicationWillFinishLaunchingNotification } {
                    "observer will-finish"
                } else {
                    "observer did-finish"
                };
                register_at(self.mtm(), point);
            }
        }
    );

    pub fn observe_launch(mtm: MainThreadMarker) {
        // SAFETY: NSObject init initializes this subclass without additional ivars.
        let observer: objc2::rc::Retained<LaunchObserver> =
            unsafe { msg_send![LaunchObserver::alloc(mtm), init] };
        // SAFETY: Both AppKit notifications carry NSNotification, matching the
        // selector above. No object filter is needed for these launch-only names.
        unsafe {
            let center = NSNotificationCenter::defaultCenter();
            for name in [
                NSApplicationWillFinishLaunchingNotification,
                NSApplicationDidFinishLaunchingNotification,
            ] {
                center.addObserver_selector_name_object(
                    &observer,
                    sel!(applicationWillFinishLaunching:),
                    Some(name),
                    None,
                );
            }
        }
        // NotificationCenter does not own the observer. Keep it alive for the
        // process lifetime, like the handler; main installs exactly one observer.
        std::mem::forget(observer);
    }

    pub fn register(mtm: MainThreadMarker) {
        register_at(mtm, "viewer fallback");
    }

    fn register_at(mtm: MainThreadMarker, point: &str) {
        static REGISTERED: Once = Once::new();
        let mut registered = false;
        REGISTERED.call_once(|| {
            // SAFETY: NSObject's init returns an initialized instance of this class.
            let handler: objc2::rc::Retained<OpenDocumentsHandler> =
                unsafe { msg_send![OpenDocumentsHandler::alloc(mtm), init] };
            // SAFETY: The selector is defined above with the required descriptor
            // arguments. The manager does not retain its handler, so deliberately
            // leak one retained instance for the process-long registration.
            unsafe {
                NSAppleEventManager::sharedAppleEventManager()
                    .setEventHandler_andSelector_forEventClass_andEventID(
                        &handler,
                        sel!(handleOpenDocuments:withReplyEvent:),
                        kCoreEventClass,
                        kAEOpenDocuments,
                    );
            }
            std::mem::forget(handler);
            registered = true;
        });
        if trace_enabled() {
            eprintln!(
                "{}",
                registration_trace(point, registered, std::process::id())
            );
        }
    }

    pub fn attach_context(ctx: &egui::Context) {
        let _ = CONTEXT.set(ctx.clone());
    }

    pub fn drain(selected: Option<&Path>) -> Option<PathBuf> {
        DOCUMENTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(selected)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn non_file_descriptor_is_ignored() {
            assert_eq!(
                descriptor_path(&NSAppleEventDescriptor::nullDescriptor()),
                None
            );
            for text in [
                "https://example.com/image.png",
                "relative.png",
                "file://remote/tmp/a.png",
            ] {
                let descriptor = NSAppleEventDescriptor::descriptorWithString(
                    &objc2_foundation::NSString::from_str(text),
                );
                assert_eq!(descriptor_path(&descriptor), None, "{text}");
            }
        }

        #[test]
        fn native_file_and_text_descriptors_extract_local_paths() {
            use objc2_foundation::{NSString, NSURL};
            let url = NSURL::URLWithString(&NSString::from_str("file:///tmp/a%20b.png")).unwrap();
            let descriptor = NSAppleEventDescriptor::descriptorWithFileURL(&url);
            assert_eq!(
                descriptor_path(&descriptor),
                Some(PathBuf::from("/tmp/a b.png"))
            );
            for text in ["file:///tmp/a%20b.png", "/tmp/a b.png"] {
                let descriptor =
                    NSAppleEventDescriptor::descriptorWithString(&NSString::from_str(text));
                assert_eq!(
                    descriptor_path(&descriptor),
                    Some(PathBuf::from("/tmp/a b.png"))
                );
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub use native::{attach_context, drain, observe_launch, register, trace_open};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_trace_identifies_point_and_once_outcome() {
        for point in [
            "observer will-finish",
            "observer did-finish",
            "viewer fallback",
        ] {
            for (registered, status) in [(true, "registered"), (false, "skipped already-done")] {
                assert_eq!(
                    registration_trace(point, registered, 42),
                    format!("GLANCE_TRACE_OPEN {point} aevt/odoc registration {status} pid=42")
                );
            }
        }
    }

    #[test]
    fn event_trace_counts_only_this_batch_without_consuming_queue() {
        let mut queue = DocumentQueue::default();
        queue.push("/tmp/older.png".into());
        let start = queue.0.len();
        queue.push("/tmp/a b.png".into());
        queue.push("/tmp/second.png".into());
        assert_eq!(
            event_trace(3, &queue.0[start..]),
            "GLANCE_TRACE_OPEN descriptors=3 queued=2 first=Some(\"/tmp/a b.png\")"
        );
        assert_eq!(
            event_trace(0, &[]),
            "GLANCE_TRACE_OPEN descriptors=0 queued=0 first=None"
        );
        assert_eq!(queue.drain(None), Some(PathBuf::from("/tmp/older.png")));
        assert_eq!(queue.drain(None), None);
    }

    #[test]
    fn document_references_are_local_absolute_paths() {
        assert_eq!(
            document_path(Some("file:///tmp/a%20b.png"), None),
            Some(PathBuf::from("/tmp/a b.png"))
        );
        assert_eq!(
            document_path(None, Some("file://localhost/tmp/a.png")),
            Some(PathBuf::from("/tmp/a.png"))
        );
        assert_eq!(
            document_path(None, Some("/tmp/a.png")),
            Some(PathBuf::from("/tmp/a.png"))
        );
        for reference in [
            "https://example.com/a.png",
            "file://remote/tmp/a.png",
            "relative.png",
            "",
        ] {
            assert_eq!(document_path(Some(reference), Some(reference)), None);
        }
        assert_eq!(document_path(None, None), None);
    }

    #[test]
    fn native_url_precedes_string_fallback() {
        assert_eq!(
            document_path(Some("file:///tmp/first.png"), Some("/tmp/second.png")),
            Some(PathBuf::from("/tmp/first.png"))
        );
        assert_eq!(
            document_path(None, Some("/tmp/fallback.png")),
            Some(PathBuf::from("/tmp/fallback.png"))
        );
    }

    #[test]
    fn queue_drains_first_path_in_arrival_order_and_clears_batch() {
        let mut queue = DocumentQueue::default();
        assert_eq!(queue.drain(None), None);
        queue.push("/tmp/first.png".into());
        queue.push("/tmp/second.png".into());
        assert_eq!(queue.drain(None), Some(PathBuf::from("/tmp/first.png")));
        assert_eq!(queue.drain(None), None);
        queue.push("/tmp/third.png".into());
        assert_eq!(queue.drain(None), Some(PathBuf::from("/tmp/third.png")));
    }

    #[test]
    fn queue_does_not_reopen_selected_path() {
        let mut queue = DocumentQueue::default();
        queue.push("/tmp/selected.png".into());
        queue.push("/tmp/selected.png".into());
        assert_eq!(queue.drain(Some(Path::new("/tmp/selected.png"))), None);
        assert_eq!(queue.drain(None), None);
    }
}
