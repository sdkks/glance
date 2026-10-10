use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    process::{Child, Command},
};

use crate::{decode::Pixels, transform::Transform};

pub const PASTE_ERROR: &str = "Paste requires a local file URL or absolute path; remote URLs and raw bitmaps are not supported.";

pub fn local_path(path: &Path) -> Option<PathBuf> {
    let bytes = path.as_os_str().as_encoded_bytes();
    (path.is_absolute() && !bytes.starts_with(b"//") && !bytes.contains(&0))
        .then(|| path.to_path_buf())
}

pub fn parse_reference(reference: &str) -> Option<PathBuf> {
    let path = if let Some(url) = reference.strip_prefix("file://") {
        let (authority, url) = url.split_at(url.find('/')?);
        if authority.contains('%') || !matches!(authority, "" | "localhost") {
            return None;
        }
        if url.contains(['?', '#', '\r', '\n']) {
            return None;
        }
        let mut decoded = Vec::new();
        let mut bytes = url.bytes();
        while let Some(byte) = bytes.next() {
            if byte == b'%' {
                let high = char::from(bytes.next()?).to_digit(16)?;
                let low = char::from(bytes.next()?).to_digit(16)?;
                decoded.push((high * 16 + low) as u8);
            } else {
                decoded.push(byte);
            }
        }
        let decoded = String::from_utf8(decoded).ok()?;
        PathBuf::from(decoded)
    } else {
        PathBuf::from(reference)
    };
    local_path(&path)
}

pub fn first_reference(text: &str) -> Option<PathBuf> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .find_map(parse_reference)
}

// Keep the clipboard object alive in the UI owner: Linux selection ownership
// has no after-exit persistence guarantee. Native file lists take precedence.
#[cfg(target_os = "macos")]
fn native_reference() -> Option<PathBuf> {
    use objc2::{ClassType, rc::autoreleasepool};
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::{NSArray, NSURL};
    autoreleasepool(|_| {
        let board = NSPasteboard::generalPasteboard();
        let classes = NSArray::from_slice(&[NSURL::class()]);
        // SAFETY: Only NSURL (which implements NSPasteboardReading) is requested,
        // with no options dictionary. Objects are retained inside this pool and
        // dynamically checked before use. The caller is the UI owner thread.
        let objects = unsafe { board.readObjectsForClasses_options(&classes, None) }?;
        objects.iter().find_map(|object| {
            let url = object.downcast::<NSURL>().ok()?;
            // Do not use url.path(): it drops a remote file URL's authority.
            parse_reference(&url.absoluteString()?.to_string())
        })
    })
}

pub fn paste(clipboard: &mut arboard::Clipboard) -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    if let Some(path) = native_reference() {
        return Ok(path);
    }
    #[cfg(not(target_os = "macos"))]
    if let Ok(files) = clipboard.get().file_list()
        && let Some(path) = files.iter().find_map(|path| local_path(path))
    {
        return Ok(path);
    }
    let text = clipboard
        .get_text()
        .map_err(|error| format!("{PASTE_ERROR} Clipboard: {error}"))?;
    first_reference(&text).ok_or_else(|| PASTE_ERROR.to_owned())
}

pub fn copy_image(
    clipboard: &mut arboard::Clipboard,
    pixels: &Pixels,
    transform: Transform,
) -> Result<(), String> {
    let copy = transform
        .materialize(pixels)
        .map_err(|error| error.to_string())?;
    clipboard
        .set_image(arboard::ImageData {
            width: copy.size[0],
            height: copy.size[1],
            bytes: Cow::Owned(copy.rgba),
        })
        .map_err(|error| {
            format!(
                "Copy Image failed; clipboard unavailable or image could not be encoded: {error}"
            )
        })
}

#[derive(Clone, Copy)]
pub enum Desktop {
    Mac,
    Linux,
}

pub fn reveal_command(path: &Path, desktop: Desktop) -> Result<Command, String> {
    let path = local_path(path)
        .ok_or_else(|| "Reveal requires an absolute local file path.".to_owned())?;
    let mut command = match desktop {
        Desktop::Mac => {
            let mut command = Command::new("open");
            command.arg("-R").arg(path);
            command
        }
        Desktop::Linux => {
            let mut command = Command::new("xdg-open");
            command.arg(
                path.parent()
                    .ok_or_else(|| "Reveal could not find the containing folder.".to_owned())?,
            );
            command
        }
    };
    command.stdin(std::process::Stdio::null());
    Ok(command)
}

pub fn reveal(path: &Path) -> Result<Child, String> {
    let desktop = if cfg!(target_os = "macos") {
        Desktop::Mac
    } else {
        Desktop::Linux
    };
    reveal_command(path, desktop)?.spawn().map_err(|error| {
        format!("Reveal failed; check that the system file manager is installed: {error}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_reference_acceptance_matrix() {
        for (input, expected) in [
            ("file:///tmp/a%20b%23%C3%A9.png", "/tmp/a b#é.png"),
            ("file://localhost/tmp/a.png", "/tmp/a.png"),
            ("/tmp/a%20b.png", "/tmp/a%20b.png"),
            ("/tmp/space and ü.png", "/tmp/space and ü.png"),
        ] {
            assert_eq!(parse_reference(input), Some(PathBuf::from(expected)));
        }
        for input in [
            "https://example.com/a.png",
            "file://remote/tmp/a.png",
            "file:relative",
            "relative.png",
            "garbage",
            "",
            "//remote/a.png",
            "file:////remote/a.png",
            "file://%2F%2Fremote/a.png",
            "file:///tmp/a%00.png",
            "file:///tmp/%GG",
            "file:///tmp/%",
            "file:///tmp/a.png?query",
            "file:///tmp/a.png#fragment",
        ] {
            assert_eq!(parse_reference(input), None, "{input}");
        }
    }
    #[test]
    fn file_url_authority_is_exact_and_not_percent_decoded() {
        assert_eq!(
            parse_reference("file://localhost/tmp/x.png"),
            Some(PathBuf::from("/tmp/x.png"))
        );
        for input in [
            "file://localhost.evil/tmp/x.png",
            "file://localhost@evil/tmp/x.png",
            "file://localhost:8080/tmp/x.png",
            "file://%6Cocalhost/tmp/x.png",
            "file://local%68ost/tmp/x.png",
            "file://user@localhost/tmp/x.png",
            "file://localhost",
        ] {
            assert_eq!(parse_reference(input), None, "{input}");
        }
    }
    #[test]
    fn first_local_entry_not_first_remote_or_comment() {
        assert_eq!(
            first_reference(
                "# list\r\nhttps://example.com/a\r\nfile://remote/a\r\nfile:///tmp/first.png\r\n/tmp/second.png"
            ),
            Some(PathBuf::from("/tmp/first.png"))
        );
        assert_eq!(first_reference("raw bitmap\nrelative.png"), None);
        assert_eq!(local_path(Path::new("remote/a")), None);
        assert_eq!(local_path(Path::new("//remote/a")), None);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn nsurl_authority_survives_validation() {
        use objc2_foundation::{NSString, NSURL};
        for (reference, expected) in [
            ("file://remote/tmp/a.png", None),
            ("https://example.com/a.png", None),
            ("file:///tmp/a%20b.png", Some(PathBuf::from("/tmp/a b.png"))),
        ] {
            let url = NSURL::URLWithString(&NSString::from_str(reference)).unwrap();
            assert_eq!(
                parse_reference(&url.absoluteString().unwrap().to_string()),
                expected
            );
        }
    }

    #[test]
    fn reveal_is_separate_argv_not_shell_code() {
        let path = Path::new("/tmp/a folder/$(touch BAD);' ü.png");
        let mac = reveal_command(path, Desktop::Mac).unwrap();
        assert_eq!(mac.get_program(), "open");
        assert_eq!(
            mac.get_args().collect::<Vec<_>>(),
            [std::ffi::OsStr::new("-R"), path.as_os_str()]
        );
        let linux = reveal_command(path, Desktop::Linux).unwrap();
        assert_eq!(linux.get_program(), "xdg-open");
        assert_eq!(
            linux.get_args().collect::<Vec<_>>(),
            [path.parent().unwrap().as_os_str()]
        );
        assert!(reveal_command(Path::new("relative"), Desktop::Mac).is_err());
    }
}
