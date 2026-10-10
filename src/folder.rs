use std::{cmp::Ordering, ffi::OsStr, fs, io, path::PathBuf};

#[derive(Debug)]
pub struct Folder {
    pub files: Vec<PathBuf>,
    pub selected: Option<usize>,
}

impl Folder {
    pub fn open(path: PathBuf) -> io::Result<Self> {
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()?.join(path)
        };
        let directory = path.is_dir();
        let parent = if directory {
            path.as_path()
        } else {
            path.parent().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "file has no containing folder")
            })?
        };
        let mut files = Vec::new();
        for entry in fs::read_dir(parent)? {
            let entry = entry?;
            if hidden_entry(&entry) || !eligible(&entry.path()) {
                continue;
            }
            // Follow file symlinks, but never descend into directories.
            if entry.path().is_file() {
                files.push(entry.path());
            }
        }
        // Explicit opens remain selected even when hidden, missing, or unsupported.
        if !directory && !files.contains(&path) {
            files.push(path.clone());
        }
        files.sort_by(|a, b| {
            compare_names(
                a.file_name().unwrap_or_default(),
                b.file_name().unwrap_or_default(),
            )
        });
        let selected = if directory {
            (!files.is_empty()).then_some(0)
        } else {
            files.iter().position(|file| file == &path)
        };
        Ok(Self { files, selected })
    }

    pub fn selected_path(&self) -> Option<&PathBuf> {
        self.selected.and_then(|index| self.files.get(index))
    }

    pub fn navigate(&mut self, direction: Direction) -> bool {
        let Some(index) = self.selected else {
            return false;
        };
        let next = match direction {
            Direction::Previous => index.saturating_sub(1),
            Direction::Next => index
                .saturating_add(1)
                .min(self.files.len().saturating_sub(1)),
            Direction::First => 0,
            Direction::Last => self.files.len().saturating_sub(1),
        };
        self.selected = Some(next);
        next != index
    }
}

#[derive(Clone, Copy)]
pub enum Direction {
    Previous,
    Next,
    First,
    Last,
}

pub fn eligible(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|extension| {
        [b"jpg".as_slice(), b"jpeg", b"png"]
            .iter()
            .any(|allowed| extension.as_encoded_bytes().eq_ignore_ascii_case(allowed))
    })
}

fn hidden_entry(entry: &fs::DirEntry) -> bool {
    #[cfg(target_os = "macos")]
    let flags = {
        use std::os::macos::fs::MetadataExt;
        entry
            .metadata()
            .map(|metadata| metadata.st_flags())
            .unwrap_or(0)
    };
    #[cfg(not(target_os = "macos"))]
    let flags = 0;
    hidden_name_or_flags(&entry.file_name(), flags)
}

fn hidden_name_or_flags(name: &OsStr, flags: u32) -> bool {
    const UF_HIDDEN: u32 = 0x8000;
    name.as_encoded_bytes().starts_with(b".") || flags & UF_HIDDEN != 0
}

fn compare_names(a: &OsStr, b: &OsStr) -> Ordering {
    natural_compare(a.as_encoded_bytes(), b.as_encoded_bytes()).then_with(|| a.cmp(b))
}

// Compare digit runs by length and bytes, never by parsing a bounded integer.
fn natural_compare(mut a: &[u8], mut b: &[u8]) -> Ordering {
    while !a.is_empty() && !b.is_empty() {
        let order = if a[0].is_ascii_digit() && b[0].is_ascii_digit() {
            let a_end = a
                .iter()
                .position(|c| !c.is_ascii_digit())
                .unwrap_or(a.len());
            let b_end = b
                .iter()
                .position(|c| !c.is_ascii_digit())
                .unwrap_or(b.len());
            let a_run = &a[..a_end];
            let b_run = &b[..b_end];
            let a_number = &a_run[a_run.iter().position(|&c| c != b'0').unwrap_or(a_end)..];
            let b_number = &b_run[b_run.iter().position(|&c| c != b'0').unwrap_or(b_end)..];
            let order = a_number
                .len()
                .cmp(&b_number.len())
                .then_with(|| a_number.cmp(b_number));
            a = &a[a_end..];
            b = &b[b_end..];
            order
        } else {
            let order = a[0].to_ascii_lowercase().cmp(&b[0].to_ascii_lowercase());
            a = &a[1..];
            b = &b[1..];
            order
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    a.len().cmp(&b.len())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "glance-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, AtomicOrdering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn hidden_names_and_finder_flags() {
        assert!(hidden_name_or_flags(OsStr::new(".hidden.png"), 0));
        assert!(hidden_name_or_flags(OsStr::new("image.png"), 0x8000));
        assert!(hidden_name_or_flags(OsStr::new("image.png"), 0x8002));
        assert!(!hidden_name_or_flags(OsStr::new("image.png"), 0x0002));
        assert!(!hidden_name_or_flags(OsStr::new("image.png"), 0));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finder_hidden_file_is_excluded_unless_explicitly_opened() {
        let dir = TempDir::new();
        let hidden = dir.0.join("hidden.png");
        let visible = dir.0.join("visible.png");
        fs::write(&hidden, b"image").unwrap();
        fs::write(&visible, b"image").unwrap();
        let status = std::process::Command::new("/usr/bin/chflags")
            .arg("hidden")
            .arg(&hidden)
            .status()
            .unwrap();
        assert!(status.success());

        let folder = Folder::open(dir.0.clone()).unwrap();
        assert_eq!(folder.files.as_slice(), std::slice::from_ref(&visible));
        let folder = Folder::open(hidden.clone()).unwrap();
        assert_eq!(folder.selected_path(), Some(&hidden));
        assert_eq!(folder.files, [hidden, visible]);
    }

    #[test]
    fn natural_order_and_exact_ties() {
        let mut names = [
            "image10.png",
            "image2.png",
            "Image2.png",
            "image02.png",
            "image1.png",
        ];
        names.sort_by(|a, b| compare_names(OsStr::new(a), OsStr::new(b)));
        assert_eq!(
            names,
            [
                "image1.png",
                "Image2.png",
                "image02.png",
                "image2.png",
                "image10.png"
            ]
        );
        assert_eq!(
            natural_compare(
                b"a99999999999999999999999999",
                b"a100000000000000000000000000"
            ),
            Ordering::Less
        );
        assert_eq!(natural_compare(b"a000", b"a0"), Ordering::Equal);
    }

    #[test]
    fn listing_is_filtered_nonrecursive_and_keeps_corrupt_files() {
        let dir = TempDir::new();
        for name in [
            "image10.PNG",
            "image2.JpEg",
            ".hidden.jpg",
            "notes.txt",
            "bad.jpg",
        ] {
            fs::write(dir.0.join(name), b"not image content").unwrap();
        }
        fs::create_dir(dir.0.join("nested.png")).unwrap();
        fs::write(dir.0.join("nested.png/child.png"), b"image").unwrap();
        let folder = Folder::open(dir.0.clone()).unwrap();
        let names: Vec<_> = folder
            .files
            .iter()
            .map(|file| file.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, ["bad.jpg", "image2.JpEg", "image10.PNG"]);
        assert_eq!(folder.selected, Some(0));
    }

    #[test]
    fn explicit_hidden_and_missing_files_stay_selected_and_navigation_stops() {
        let dir = TempDir::new();
        fs::write(dir.0.join("other.png"), b"bad").unwrap();
        for name in [".hidden.png", "missing.png"] {
            let mut folder = Folder::open(dir.0.join(name)).unwrap();
            assert_eq!(folder.selected_path(), Some(&dir.0.join(name)));
            folder.navigate(Direction::First);
            assert!(!folder.navigate(Direction::Previous));
            folder.navigate(Direction::Last);
            assert!(!folder.navigate(Direction::Next));
            assert_eq!(folder.files.len(), 2);
        }
    }

    #[test]
    fn empty_folder_has_no_selection() {
        let dir = TempDir::new();
        let mut folder = Folder::open(dir.0.clone()).unwrap();
        assert!(folder.selected_path().is_none());
        assert!(!folder.navigate(Direction::Last));
    }
}
