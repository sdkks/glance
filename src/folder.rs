use std::{
    cmp::Ordering, collections::HashMap, ffi::OsStr, fs, io, path::PathBuf, time::SystemTime,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileVersion {
    pub path: PathBuf,
    size: u64,
    modified: SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

impl FileVersion {
    /// Decode/playback guard: ctime-only bookkeeping does not change image content.
    /// Keep full equality (including path and ctime) for conservative cache reuse.
    pub fn same_content(&self, other: &Self) -> bool {
        if self.size != other.size || self.modified != other.modified {
            return false;
        }
        #[cfg(unix)]
        {
            (self.identity.0, self.identity.1) == (other.identity.0, other.identity.1)
        }
        #[cfg(not(unix))]
        {
            true
        }
    }

    pub fn read(path: &std::path::Path) -> io::Result<Self> {
        let metadata = fs::metadata(path)?;
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            path: path.to_path_buf(),
            size: metadata.len(),
            modified: metadata.modified()?,
            #[cfg(unix)]
            identity: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    NameAscending,
    NameDescending,
    SizeLargest,
    SizeSmallest,
    ModifiedNewest,
    ModifiedOldest,
}

impl Sort {
    pub const ALL: [(Self, &'static str); 6] = [
        (Self::NameAscending, "Name A→Z"),
        (Self::NameDescending, "Name Z→A"),
        (Self::SizeLargest, "Size largest first"),
        (Self::SizeSmallest, "Size smallest first"),
        (Self::ModifiedNewest, "Modified newest first"),
        (Self::ModifiedOldest, "Modified oldest first"),
    ];
}

#[derive(Clone, Debug, Default)]
struct FileMetadata {
    size: Option<u64>,
    modified: Option<SystemTime>,
}

#[derive(Clone, Debug)]
pub struct Folder {
    pub files: Vec<PathBuf>,
    pub selected: Option<usize>,
    versions: HashMap<PathBuf, FileVersion>,
    parent: PathBuf,
    explicit: Option<PathBuf>,
    pub sort: Sort,
}

impl Folder {
    #[cfg(test)]
    pub fn open(path: PathBuf) -> io::Result<Self> {
        Self::open_sorted(path, Sort::default())
    }

    pub fn open_sorted(path: PathBuf, sort: Sort) -> io::Result<Self> {
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()?.join(path)
        };
        let directory = path.is_dir();
        let parent = if directory {
            path.clone()
        } else {
            path.parent()
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "file has no containing folder")
                })?
                .to_path_buf()
        };
        let explicit = (!directory).then_some(path.clone());
        let mut folder = Self {
            files: Vec::new(),
            selected: None,
            versions: HashMap::new(),
            parent,
            explicit,
            sort,
        };
        folder.files = folder.snapshot(true)?;
        folder.update_versions();
        folder.selected = if directory {
            (!folder.files.is_empty()).then_some(0)
        } else {
            folder.files.iter().position(|file| file == &path)
        };
        Ok(folder)
    }

    fn snapshot(&self, initial: bool) -> io::Result<Vec<PathBuf>> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.parent)? {
            let entry = entry?;
            let path = entry.path();
            let explicit = self.explicit.as_ref() == Some(&path);
            if (!explicit && (hidden_entry(&entry) || !eligible(&path))) || !path.is_file() {
                continue;
            }
            entries.push((path.clone(), metadata(&path)));
        }
        if initial
            && let Some(path) = &self.explicit
            && !entries.iter().any(|(file, _)| file == path)
        {
            entries.push((path.clone(), metadata(path)));
        }
        entries.sort_by(|(a, am), (b, bm)| compare_files(a, am, b, bm, self.sort));
        Ok(entries.into_iter().map(|(path, _)| path).collect())
    }

    fn update_versions(&mut self) {
        self.versions = self
            .files
            .iter()
            .filter_map(|path| FileVersion::read(path).ok().map(|v| (path.clone(), v)))
            .collect();
    }

    pub fn version(&self, path: &PathBuf) -> Option<&FileVersion> {
        self.versions.get(path)
    }

    pub fn parent(&self) -> &std::path::Path {
        &self.parent
    }

    pub fn selected_path(&self) -> Option<&PathBuf> {
        self.selected.and_then(|index| self.files.get(index))
    }

    // Old ordering is the tombstone anchor, including when several neighbors vanished.
    pub fn refresh(&mut self, direction: Option<Direction>, sort: Sort) -> io::Result<bool> {
        let old_path = self.selected_path().cloned();
        let old_index = self.selected;
        let old_sort = self.sort;
        self.sort = sort;
        let files = match self.snapshot(false) {
            Ok(files) => files,
            Err(error) => {
                self.sort = old_sort;
                return Err(error);
            }
        };
        let same = old_path
            .as_ref()
            .and_then(|path| files.iter().position(|file| file == path));
        let selected = match direction {
            Some(Direction::First) => (!files.is_empty()).then_some(0),
            Some(Direction::Last) => files.len().checked_sub(1),
            Some(direction) if same.is_some() => {
                let index = same.unwrap_or(0);
                Some(match direction {
                    Direction::Previous => index.saturating_sub(1),
                    _ => index.saturating_add(1).min(files.len().saturating_sub(1)),
                })
            }
            _ if same.is_some() => same,
            _ => old_index
                .and_then(|index| {
                    let previous = matches!(direction, Some(Direction::Previous));
                    let candidates: Vec<_> = if previous {
                        self.files[..index]
                            .iter()
                            .rev()
                            .chain(self.files[index + 1..].iter())
                            .collect()
                    } else {
                        self.files[index + 1..]
                            .iter()
                            .chain(self.files[..index].iter().rev())
                            .collect()
                    };
                    candidates
                        .into_iter()
                        .find_map(|path| files.iter().position(|file| file == path))
                })
                .or_else(|| (!files.is_empty()).then_some(0)),
        };
        self.files = files;
        self.update_versions();
        self.selected = selected;
        Ok(self.selected_path() != old_path.as_ref())
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

fn metadata(path: &std::path::Path) -> FileMetadata {
    fs::metadata(path)
        .map(|m| FileMetadata {
            size: Some(m.len()),
            modified: m.modified().ok(),
        })
        .unwrap_or_default()
}

fn compare_files(
    a: &std::path::Path,
    am: &FileMetadata,
    b: &std::path::Path,
    bm: &FileMetadata,
    sort: Sort,
) -> Ordering {
    let names = || {
        compare_names(
            a.file_name().unwrap_or_default(),
            b.file_name().unwrap_or_default(),
        )
    };
    match sort {
        Sort::NameAscending => names(),
        Sort::NameDescending => names().reverse(),
        Sort::SizeLargest => bm.size.cmp(&am.size).then_with(names),
        Sort::SizeSmallest => am.size.cmp(&bm.size).then_with(names),
        Sort::ModifiedNewest => bm.modified.cmp(&am.modified).then_with(names),
        Sort::ModifiedOldest => am.modified.cmp(&bm.modified).then_with(names),
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
    crate::formats::format(path).is_some()
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

    #[cfg(unix)]
    pub fn change_metadata_only(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        let before = FileVersion::read(path).unwrap();
        let mode = fs::metadata(path).unwrap().permissions().mode();
        fs::set_permissions(path, fs::Permissions::from_mode(mode ^ 0o100)).unwrap();
        let after = FileVersion::read(path).unwrap();
        assert_eq!(before.size, after.size);
        assert_eq!(before.modified, after.modified);
        assert_eq!(before.identity.0, after.identity.0);
        assert_eq!(before.identity.1, after.identity.1);
        assert_ne!(before, after, "chmod must change ctime for this regression");
    }

    #[test]
    fn content_comparison_keeps_size_mtime_and_file_identity() {
        let dir = TempDir::new();
        let path = dir.0.join("image.png");
        fs::write(&path, b"image").unwrap();
        let version = FileVersion::read(&path).unwrap();
        let mut other = version.clone();
        assert!(version.same_content(&other));
        other.size += 1;
        assert!(!version.same_content(&other));
        other = version.clone();
        other.modified += std::time::Duration::from_secs(1);
        assert!(!version.same_content(&other));
        #[cfg(unix)]
        {
            for component in [0, 1] {
                other = version.clone();
                if component == 0 {
                    other.identity.0 += 1;
                } else {
                    other.identity.1 += 1;
                }
                assert!(!version.same_content(&other));
            }
            other = version.clone();
            other.identity.2 += 1;
            other.identity.3 += 1;
            assert!(version.same_content(&other));
            assert_ne!(version, other, "cache equality must remain strict");
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
    fn refresh_preserves_identity_adds_files_and_keeps_corrupt_entries() {
        let dir = TempDir::new();
        let current = dir.0.join("image2.png");
        fs::write(&current, b"corrupt").unwrap();
        fs::write(dir.0.join("image10.png"), b"longer corrupt").unwrap();
        let mut folder = Folder::open(current.clone()).unwrap();
        fs::write(dir.0.join("image1.png"), b"bad").unwrap();
        assert!(!folder.refresh(None, Sort::NameAscending).unwrap());
        assert_eq!(folder.selected, Some(1));
        assert_eq!(folder.files.len(), 3);
        for (sort, _) in Sort::ALL {
            assert!(!folder.refresh(None, sort).unwrap());
            assert_eq!(folder.selected_path(), Some(&current));
        }
        assert_eq!(folder.sort, Sort::ModifiedOldest);
    }

    #[test]
    fn deletion_uses_old_directional_neighbors_even_after_multiple_removals() {
        for direction in [Direction::Previous, Direction::Next] {
            let dir = TempDir::new();
            for n in 1..=7 {
                fs::write(dir.0.join(format!("{n}.png")), b"bad").unwrap();
            }
            let mut folder = Folder::open(dir.0.join("4.png")).unwrap();
            for n in 3..=5 {
                fs::remove_file(dir.0.join(format!("{n}.png"))).unwrap();
            }
            fs::write(dir.0.join("3-new.png"), b"bad").unwrap();
            assert!(
                folder
                    .refresh(Some(direction), Sort::NameAscending)
                    .unwrap()
            );
            let expected = if matches!(direction, Direction::Previous) {
                "2.png"
            } else {
                "6.png"
            };
            assert_eq!(folder.selected_path(), Some(&dir.0.join(expected)));
        }
    }

    #[test]
    fn deletion_at_boundary_uses_nearest_other_neighbor_and_empty_is_safe() {
        let dir = TempDir::new();
        for name in ["1.png", "2.png"] {
            fs::write(dir.0.join(name), b"bad").unwrap();
        }
        let mut folder = Folder::open(dir.0.join("2.png")).unwrap();
        fs::remove_file(dir.0.join("2.png")).unwrap();
        folder
            .refresh(Some(Direction::Next), Sort::default())
            .unwrap();
        assert_eq!(folder.selected_path(), Some(&dir.0.join("1.png")));
        fs::remove_file(dir.0.join("1.png")).unwrap();
        folder
            .refresh(Some(Direction::Previous), Sort::default())
            .unwrap();
        assert_eq!(folder.selected_path(), None);
        fs::write(dir.0.join("new.png"), b"bad").unwrap();
        folder
            .refresh(Some(Direction::Next), Sort::default())
            .unwrap();
        assert_eq!(folder.selected_path(), Some(&dir.0.join("new.png")));
    }

    #[test]
    fn metadata_directions_ties_and_snapshot_time() {
        use std::time::{Duration, UNIX_EPOCH};
        let dir = TempDir::new();
        for (name, size, seconds) in [
            ("image10.png", 10, 30),
            ("image2.png", 20, 10),
            ("image1.png", 20, 10),
            ("image02.png", 20, 10),
        ] {
            let path = dir.0.join(name);
            fs::write(&path, vec![0; size]).unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(
                    fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(seconds)),
                )
                .unwrap();
        }
        let mut folder = Folder::open(dir.0.clone()).unwrap();
        for (sort, expected) in [
            (
                Sort::NameAscending,
                ["image1.png", "image02.png", "image2.png", "image10.png"],
            ),
            (
                Sort::NameDescending,
                ["image10.png", "image2.png", "image02.png", "image1.png"],
            ),
            (
                Sort::SizeLargest,
                ["image1.png", "image02.png", "image2.png", "image10.png"],
            ),
            (
                Sort::SizeSmallest,
                ["image10.png", "image1.png", "image02.png", "image2.png"],
            ),
            (
                Sort::ModifiedNewest,
                ["image10.png", "image1.png", "image02.png", "image2.png"],
            ),
            (
                Sort::ModifiedOldest,
                ["image1.png", "image02.png", "image2.png", "image10.png"],
            ),
        ] {
            folder.refresh(None, sort).unwrap();
            let names: Vec<_> = folder
                .files
                .iter()
                .map(|p| p.file_name().unwrap().to_str().unwrap())
                .collect();
            assert_eq!(names, expected);
        }
        let path = dir.0.join("image10.png");
        fs::write(&path, vec![0; 100]).unwrap();
        folder.refresh(None, Sort::SizeLargest).unwrap();
        assert_eq!(folder.files.first(), Some(&path));
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(1)))
            .unwrap();
        folder.refresh(None, Sort::ModifiedOldest).unwrap();
        assert_eq!(folder.files.first(), Some(&path));
    }

    #[test]
    fn metadata_exact_name_ties_are_ascending_in_both_directions() {
        let metadata = FileMetadata {
            size: Some(20),
            modified: Some(SystemTime::UNIX_EPOCH),
        };
        for sort in [
            Sort::SizeLargest,
            Sort::SizeSmallest,
            Sort::ModifiedNewest,
            Sort::ModifiedOldest,
        ] {
            let mut names = [
                PathBuf::from("image2.png"),
                PathBuf::from("Image2.png"),
                PathBuf::from("image02.png"),
            ];
            names.sort_by(|a, b| compare_files(a, &metadata, b, &metadata, sort));
            assert_eq!(
                names,
                [
                    PathBuf::from("Image2.png"),
                    PathBuf::from("image02.png"),
                    PathBuf::from("image2.png")
                ]
            );
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn non_utf8_identity_is_not_lossy() {
        use std::os::unix::ffi::OsStringExt;
        let dir = TempDir::new();
        let path = dir
            .0
            .join(std::ffi::OsString::from_vec(b"image\xff.png".to_vec()));
        fs::write(&path, b"bad").unwrap();
        let mut folder = Folder::open(path.clone()).unwrap();
        folder.refresh(None, Sort::NameDescending).unwrap();
        assert_eq!(folder.selected_path(), Some(&path));
    }

    #[test]
    fn empty_folder_has_no_selection() {
        let dir = TempDir::new();
        let mut folder = Folder::open(dir.0.clone()).unwrap();
        assert!(folder.selected_path().is_none());
        assert!(!folder.navigate(Direction::Last));
    }
}
