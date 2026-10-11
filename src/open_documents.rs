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

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use eframe::egui;
    use objc2::{MainThreadOnly, define_class, msg_send, sel};
    use objc2_core_services::{
        kAEOpenDocuments, kCoreEventClass, keyDirectObject, typeAlias, typeBookmarkData, typeFSRef,
        typeFileURL,
    };
    use objc2_foundation::{
        MainThreadMarker, NSAppleEventDescriptor, NSAppleEventManager, NSObject,
    };
    use std::sync::{Mutex, OnceLock};

    static DOCUMENTS: Mutex<DocumentQueue> = Mutex::new(DocumentQueue(Vec::new()));
    static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

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
                if let Some(list) = event.paramDescriptorForKeyword(keyDirectObject) {
                    let mut queue = DOCUMENTS.lock().unwrap_or_else(|e| e.into_inner());
                    // AppleEvent list indices are one-based.
                    for index in 1..=list.numberOfItems() {
                        if let Some(path) = list.descriptorAtIndex(index)
                            .and_then(|descriptor| descriptor_path(&descriptor))
                        {
                            queue.push(path);
                        }
                    }
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

    pub fn register(mtm: MainThreadMarker) {
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
pub use native::{attach_context, drain, register};

#[cfg(test)]
mod tests {
    use super::*;

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
