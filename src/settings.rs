use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OpenNewFiles {
    #[default]
    CurrentWindow,
    NewWindow,
}

impl OpenNewFiles {
    pub const ALL: [(Self, &str); 2] = [
        (Self::CurrentWindow, "Current window"),
        (Self::NewWindow, "New window"),
    ];

    fn parse(key: &str) -> Self {
        match key {
            "new-window" => Self::NewWindow,
            _ => Self::CurrentWindow,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::CurrentWindow => "current-window",
            Self::NewWindow => "new-window",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Background {
    White,
    Grey,
    #[default]
    Black,
}

impl Background {
    pub const ALL: [(Self, &str); 3] = [
        (Self::White, "White"),
        (Self::Grey, "Grey"),
        (Self::Black, "Black"),
    ];

    fn parse(key: &str) -> Self {
        match key {
            "white" => Self::White,
            "grey" => Self::Grey,
            _ => Self::Black,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::White => "white",
            Self::Grey => "grey",
            Self::Black => "black",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::White => Self::Grey,
            Self::Grey => Self::Black,
            Self::Black => Self::White,
        }
    }

    pub fn linear(self) -> f32 {
        match self {
            Self::White => 1.0,
            Self::Grey => crate::levels::linear(128),
            // Preserve the previous shader constant exactly, not a recalculation.
            Self::Black => 0.006048833,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub preload_depth: usize,
    pub animation_rate: crate::animation::Rate,
    pub open_new_files: OpenNewFiles,
    pub background: Background,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            preload_depth: 1,
            animation_rate: Default::default(),
            open_new_files: Default::default(),
            background: Default::default(),
        }
    }
}

impl Settings {
    fn parse(text: &str) -> Result<Self, String> {
        let document = toml_edit::Document::parse(text).map_err(|e| e.to_string())?;
        let mut settings = Self::default();
        if let Some(value) = document.get("preload-depth") {
            settings.preload_depth = match value.as_integer() {
                Some(depth @ 0..=5) => depth as usize,
                _ => return Err("preload-depth must be an integer from 0 (Off) through 5".into()),
            };
        }
        settings.animation_rate = document
            .get("animation-rate")
            .and_then(|value| value.as_str())
            .map_or(Default::default(), crate::animation::Rate::parse);
        settings.open_new_files = document
            .get("open-new-files-in")
            .and_then(|value| value.as_str())
            .map_or(Default::default(), OpenNewFiles::parse);
        settings.background = document
            .get("background-color")
            .and_then(|value| value.as_str())
            .map_or(Default::default(), Background::parse);
        Ok(settings)
    }

    fn text(self) -> String {
        format!(
            "preload-depth = {}\nanimation-rate = \"{}\"\nopen-new-files-in = \"{}\"\nbackground-color = \"{}\"\n",
            self.preload_depth,
            self.animation_rate.key(),
            self.open_new_files.key(),
            self.background.key()
        )
    }
}

// OS choice and environment inputs are explicit so both paths can be tested on either host.
fn resolve_path(macos: bool, home: Option<OsString>, config: Option<OsString>) -> Option<PathBuf> {
    let home = PathBuf::from(home?);
    if !home.is_absolute() {
        return None;
    }
    let base = if macos {
        home.join("Library/Application Support")
    } else {
        config
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".config"))
    };
    Some(base.join("glance/settings.toml"))
}

fn config_path() -> Option<PathBuf> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        resolve_path(
            cfg!(target_os = "macos"),
            std::env::var_os("HOME"),
            std::env::var_os("XDG_CONFIG_HOME"),
        )
    }
    // Windows persistence is deferred; do not guess an AppData location.
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

fn debug(message: impl std::fmt::Display) {
    // No logging framework is needed for these non-modal diagnostics.
    eprintln!("[debug] Settings: {message}");
}

pub struct Store {
    path: Option<PathBuf>,
}

impl Store {
    #[cfg(test)]
    pub fn session_only() -> Self {
        Self { path: None }
    }

    pub fn load() -> (Settings, Self) {
        Self::load_at(config_path())
    }

    fn load_at(path: Option<PathBuf>) -> (Settings, Self) {
        let settings = path.as_ref().map_or_else(
            || {
                debug("no supported config location (HOME missing/invalid); session-only");
                Settings::default()
            },
            |path| {
                fs::read_to_string(path)
                    .map_err(|e| e.to_string())
                    .and_then(|text| Settings::parse(&text))
                    .unwrap_or_else(|error| {
                        debug(format!("{}: {error}; using defaults", path.display()));
                        Settings::default()
                    })
            },
        );
        debug(format!("loaded preload-depth = {}", settings.preload_depth));
        (settings, Self { path })
    }

    pub fn save(&mut self, settings: Settings) {
        if let Some(path) = &self.path
            && let Err(error) = atomic_write(path, settings.text().as_bytes())
        {
            debug(format!("{}: {error}; session-only", path.display()));
            self.path = None;
        }
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().ok_or_else(|| io::Error::other("no parent"))?;
    fs::create_dir_all(parent)?;
    // create_new prevents clobbering another instance's temporary file.
    let (temp, mut file) = loop {
        let temp = parent.join(format!(
            ".settings-{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => break (temp, file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folder::tests::TempDir;

    #[test]
    fn backgrounds_roundtrip_and_default_to_black() {
        assert_eq!(Settings::default().background, Background::Black);
        assert_eq!(Settings::parse("").unwrap().background, Background::Black);
        let dir = TempDir::new();
        let path = dir.0.join("settings.toml");
        for (background, _) in Background::ALL {
            let settings = Settings {
                preload_depth: 4,
                animation_rate: crate::animation::Rate::Exact,
                open_new_files: OpenNewFiles::NewWindow,
                background,
            };
            let mut store = Store {
                path: Some(path.clone()),
            };
            store.save(settings);
            assert_eq!(Store::load_at(Some(path.clone())).0, settings);
        }
    }

    #[test]
    fn invalid_background_preserves_other_preferences() {
        for value in ["'future'", "'WHITE'", "42", "false", "[]", "{}"] {
            let settings = Settings::parse(&format!(
                "preload-depth = 4\nanimation-rate = 'exact'\nopen-new-files-in = 'new-window'\nbackground-color = {value}\n"
            )).unwrap();
            assert_eq!(
                settings,
                Settings {
                    preload_depth: 4,
                    animation_rate: crate::animation::Rate::Exact,
                    open_new_files: OpenNewFiles::NewWindow,
                    background: Background::Black,
                }
            );
        }
    }

    #[test]
    fn backgrounds_cycle_white_grey_black_white() {
        assert_eq!(Background::White.next(), Background::Grey);
        assert_eq!(Background::Grey.next(), Background::Black);
        assert_eq!(Background::Black.next(), Background::White);
    }

    #[test]
    fn open_policy_roundtrips_and_missing_key_preserves_current_window_default() {
        let dir = TempDir::new();
        let path = dir.0.join("settings.toml");
        for (open_new_files, _) in OpenNewFiles::ALL {
            let settings = Settings {
                preload_depth: 3,
                animation_rate: crate::animation::Rate::Fps10,
                open_new_files,
                ..Default::default()
            };
            let mut store = Store {
                path: Some(path.clone()),
            };
            store.save(settings);
            assert_eq!(Store::load_at(Some(path.clone())).0, settings);
            assert!(
                fs::read_to_string(&path)
                    .unwrap()
                    .contains(open_new_files.key())
            );
        }
        fs::write(&path, "preload-depth = 4\nanimation-rate = 'exact'\n").unwrap();
        assert_eq!(
            Store::load_at(Some(path)).0,
            Settings {
                preload_depth: 4,
                animation_rate: crate::animation::Rate::Exact,
                open_new_files: OpenNewFiles::CurrentWindow,
                ..Default::default()
            }
        );
        assert_eq!(
            Settings::parse("").unwrap().open_new_files,
            OpenNewFiles::CurrentWindow
        );
    }

    #[test]
    fn invalid_open_policy_uses_default_without_resetting_other_preferences() {
        for value in ["'future'", "'NEW-WINDOW'", "42", "false", "[]", "{}"] {
            let settings = Settings::parse(&format!(
                "preload-depth = 4\nanimation-rate = 'exact'\nopen-new-files-in = {value}\n"
            ))
            .unwrap();
            assert_eq!(
                settings,
                Settings {
                    preload_depth: 4,
                    animation_rate: crate::animation::Rate::Exact,
                    open_new_files: OpenNewFiles::CurrentWindow,
                    ..Default::default()
                }
            );
        }
    }

    #[test]
    fn animation_rates_roundtrip_and_invalid_values_preserve_depth() {
        let dir = TempDir::new();
        let path = dir.0.join("settings.toml");
        for (animation_rate, _, _) in crate::animation::Rate::ALL {
            let settings = Settings {
                preload_depth: 3,
                animation_rate,
                ..Default::default()
            };
            let mut store = Store {
                path: Some(path.clone()),
            };
            store.save(settings);
            assert_eq!(Store::load_at(Some(path.clone())).0, settings);
        }
        for value in ["'future'", "42", "false", "[]", "{}"] {
            let text = format!("preload-depth = 4\nanimation-rate = {value}\n");
            fs::write(&path, text).unwrap();
            assert_eq!(
                Store::load_at(Some(path.clone())).0,
                Settings {
                    preload_depth: 4,
                    ..Default::default()
                }
            );
        }
        assert_eq!(
            Settings::parse("animation-rate = 'exact'")
                .unwrap()
                .animation_rate,
            crate::animation::Rate::Exact
        );
        assert_eq!(
            Settings::parse("preload-depth = 4").unwrap().animation_rate,
            crate::animation::Rate::default()
        );
    }

    #[test]
    fn roundtrip_all_depths() {
        let dir = TempDir::new();
        let path = dir.0.join("config/settings.toml");
        for depth in 0..=5 {
            let settings = Settings {
                preload_depth: depth,
                ..Default::default()
            };
            let mut store = Store {
                path: Some(path.clone()),
            };
            store.save(settings);
            assert_eq!(Store::load_at(Some(path.clone())).0, settings);
        }
    }

    #[test]
    fn corrupt_missing_and_invalid_values_use_defaults() {
        let dir = TempDir::new();
        let path = dir.0.join("settings.toml");
        assert_eq!(Store::load_at(Some(path.clone())).0, Settings::default());
        for text in [
            "broken",
            "preload-depth = [",
            "preload-depth = 6",
            "preload-depth = -1",
            "preload-depth = '5'",
            "preload-depth = 2\npreload-depth = 3",
        ] {
            fs::write(&path, text).unwrap();
            assert_eq!(Store::load_at(Some(path.clone())).0, Settings::default());
        }
    }

    #[test]
    fn unknown_keys_comments_and_tables_are_tolerated() {
        let settings = Settings::parse("# preferences\npreload-depth = 5 # neighbors\nfuture = 'value'\n[sort]\ndirection = 'ascending'\n").unwrap();
        assert_eq!(settings.preload_depth, 5);
        assert_eq!(
            Settings::parse("future = true").unwrap(),
            Settings::default()
        );
    }

    #[test]
    fn injected_platform_paths() {
        let home = Some(OsString::from("/home/viewer"));
        assert_eq!(
            resolve_path(false, home.clone(), None).unwrap(),
            PathBuf::from("/home/viewer/.config/glance/settings.toml")
        );
        assert_eq!(
            resolve_path(false, home.clone(), Some("relative".into())).unwrap(),
            PathBuf::from("/home/viewer/.config/glance/settings.toml")
        );
        assert_eq!(
            resolve_path(false, home.clone(), Some("/config".into())).unwrap(),
            PathBuf::from("/config/glance/settings.toml")
        );
        assert_eq!(
            resolve_path(true, home, Some("/ignored".into())).unwrap(),
            PathBuf::from("/home/viewer/Library/Application Support/glance/settings.toml")
        );
        assert!(resolve_path(false, None, Some("/config".into())).is_none());
        assert!(resolve_path(true, Some("".into()), None).is_none());
    }

    #[test]
    fn xdg_override_environment_is_restored() {
        // Run mutation in a separate, single-threaded child test process: other tests
        // and the viewer can never observe the changed environment.
        const CHILD: &str = "GLANCE_SETTINGS_ENV_TEST";
        if std::env::var_os(CHILD).is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "settings::tests::xdg_override_environment_is_restored",
                    "--test-threads=1",
                ])
                .env(CHILD, "1")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        let old = std::env::var_os("XDG_CONFIG_HOME");
        // SAFETY: this isolated child runs just this test and starts no other threads.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", "/custom-config") };
        assert_eq!(
            resolve_path(
                false,
                Some("/home/viewer".into()),
                std::env::var_os("XDG_CONFIG_HOME")
            )
            .unwrap(),
            PathBuf::from("/custom-config/glance/settings.toml")
        );
        // SAFETY: same isolated child; restore the original value before returning.
        unsafe {
            match old {
                Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }
    }

    #[test]
    fn atomic_replacement_is_complete_and_cleans_temporary_files() {
        let dir = TempDir::new();
        let path = dir.0.join("settings.toml");
        let first = Settings {
            preload_depth: 0,
            ..Default::default()
        }
        .text();
        let second = Settings {
            preload_depth: 5,
            ..Default::default()
        }
        .text();
        atomic_write(&path, first.as_bytes()).unwrap();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for n in 0..100 {
                    atomic_write(
                        &path,
                        if n % 2 == 0 {
                            second.as_bytes()
                        } else {
                            first.as_bytes()
                        },
                    )
                    .unwrap();
                }
            });
            for _ in 0..1000 {
                let observed = fs::read_to_string(&path).unwrap();
                assert!(observed == first || observed == second);
            }
        });
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[test]
    fn failed_write_disables_persistence_and_cleans_temp() {
        let dir = TempDir::new();
        let path = dir.0.join("settings.toml");
        fs::create_dir(&path).unwrap(); // rename over a directory must fail
        let mut store = Store { path: Some(path) };
        store.save(Settings {
            preload_depth: 5,
            ..Default::default()
        });
        assert!(store.path.is_none());
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }
}
