//! User-initiated default application changes, never installation-time changes.
use crate::associations::supported_types;

#[cfg(any(target_os = "linux", test))]
pub fn merge_mimeapps(existing: &str, entries: &[&str]) -> String {
    let mut output = String::new();
    let mut in_defaults = false;
    let mut found_group = false;
    let mut seen = std::collections::BTreeSet::new();
    for line in existing.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_defaults {
                append_missing(&mut output, entries, &mut seen);
            }
            in_defaults = trimmed == "[Default Applications]";
            found_group |= in_defaults;
        }
        if in_defaults
            && !trimmed.starts_with(['#', ';'])
            && let Some((key, _)) = trimmed.split_once('=')
            && entries.contains(&key.trim())
        {
            let key = key.trim();
            output.push_str(&format!("{key}=glance.desktop;\n"));
            seen.insert(key);
        } else {
            output.push_str(line);
        }
    }
    if !found_group {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str("[Default Applications]\n");
    }
    append_missing(&mut output, entries, &mut seen);
    output
}

#[cfg(any(target_os = "linux", test))]
fn append_missing<'a>(
    output: &mut String,
    entries: &[&'a str],
    seen: &mut std::collections::BTreeSet<&'a str>,
) {
    for &entry in entries {
        if seen.insert(entry) {
            if !output.ends_with('\n') {
                output.push('\n');
            }
            output.push_str(&format!("{entry}=glance.desktop;\n"));
        }
    }
}

#[cfg(target_os = "linux")]
pub fn set_default_handler() -> Result<(), String> {
    set_linux_default_handler()
}

// Compile the Linux filesystem path in macOS tests as well; desktop integration
// still requires a real Linux session and is not exercised by unit tests.
#[cfg(any(target_os = "linux", test))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn set_linux_default_handler() -> Result<(), String> {
    use std::{env, fs, io::Write, path::PathBuf};
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute());
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home.as_ref().map(|p| p.join(".config")))
        .ok_or("Cannot locate configuration: set an absolute XDG_CONFIG_HOME or HOME.")?;
    let data = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home.as_ref().map(|p| p.join(".local/share")));
    let mut data_dirs = data.into_iter().collect::<Vec<_>>();
    data_dirs.extend(env::split_paths(
        &env::var_os("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into()),
    ));
    if !data_dirs
        .iter()
        .any(|dir| dir.is_absolute() && dir.join("applications/glance.desktop").is_file())
    {
        return Err("Install glance.desktop first with make install-linux or copy it to ~/.local/share/applications (also required for AppImage).".into());
    }
    let path = config.join("mimeapps.list");
    let existing = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("Cannot read {}: {error}", path.display())),
    };
    let entries = supported_types().iter().map(|t| t.mime).collect::<Vec<_>>();
    let merged = merge_mimeapps(&existing, &entries);
    fs::create_dir_all(&config)
        .map_err(|error| format!("Cannot create {}: {error}", config.display()))?;
    let temporary = config.join(format!(".glance-mimeapps-{}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("Cannot create {}: {error}", temporary.display()))?;
    let result = (|| {
        file.write_all(merged.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, &path)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Cannot update {}: {error}", path.display()));
    }
    Ok(())
}

// Called only by the worker. NSWorkspace invokes completion blocks asynchronously;
// waiting for each reply here never stalls egui's main thread.
#[cfg(target_os = "macos")]
pub fn set_default_handler() -> Result<(), String> {
    use block2::RcBlock;
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSBundle, NSError, NSOperatingSystemVersion, NSProcessInfo, NSString};
    use objc2_uniform_type_identifiers::UTType;
    autoreleasepool(|_| {
        let version = NSOperatingSystemVersion {
            majorVersion: 12,
            minorVersion: 0,
            patchVersion: 0,
        };
        if !NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(version) {
            return Err("This action requires macOS 12 or newer. On macOS 11, use Finder Get Info → Open with → Change All… instead.".into());
        }
        let bundle = NSBundle::mainBundle();
        let url = bundle.bundleURL();
        if bundle.bundleIdentifier().is_none()
            || !url.path().is_some_and(|p| p.to_string().ends_with(".app"))
        {
            return Err("Run the installed Glance.app bundle before setting defaults; or use Finder Get Info → Change All….".into());
        }
        // Resolve every UTI before changing any associations.
        let mut types = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for entry in supported_types() {
            if seen.insert(entry.uti) {
                let content = UTType::typeWithIdentifier(&NSString::from_str(entry.uti))
                    .ok_or_else(|| {
                        format!(
                            "Unknown content type {}. Install the packaged Glance.app first.",
                            entry.uti
                        )
                    })?;
                types.push((entry.uti, content));
            }
        }
        let workspace = NSWorkspace::sharedWorkspace();
        let mut failures = Vec::new();
        for (identifier, content) in types {
            let (sender, receiver) = std::sync::mpsc::channel();
            let completion = RcBlock::new(move |error: *mut NSError| {
                // SAFETY: NSWorkspace supplies either null (success) or a live
                // NSError for the callback duration. Only its owned text escapes.
                let result = unsafe { error.as_ref() }.map_or(Ok(()), |error| {
                    Err(error.localizedDescription().to_string())
                });
                let _ = sender.send(result);
            });
            workspace.setDefaultApplicationAtURL_toOpenContentType_completionHandler(
                &url,
                &content,
                Some(&completion),
            );
            match receiver.recv_timeout(std::time::Duration::from_secs(60)) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => failures.push(format!("{identifier}: {error}")),
                Err(error) => {
                    return Err(format!(
                        "Default application result unavailable ({identifier}: {error}); some associations may have changed. Check Finder Get Info before retrying."
                    ));
                }
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Some associations could not be changed; others may have changed: {}",
                failures.join("; ")
            ))
        }
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn set_default_handler() -> Result<(), String> {
    Err("Setting the default image viewer is not supported on this platform.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    const ENTRIES: &[&str] = &["image/png", "image/jpeg"];

    #[test]
    fn empty_file_and_terminated_values() {
        assert_eq!(
            merge_mimeapps("", ENTRIES),
            "[Default Applications]\nimage/png=glance.desktop;\nimage/jpeg=glance.desktop;\n"
        );
    }
    #[test]
    fn update_in_place_and_preserve_other_keys_comments_groups() {
        let before = "# header\n[Default Applications]\n# keep\ntext/plain=editor.desktop;\nimage/png=old.desktop;\n[Added Associations]\nimage/png=other.desktop;\n";
        let after = "# header\n[Default Applications]\n# keep\ntext/plain=editor.desktop;\nimage/png=glance.desktop;\nimage/jpeg=glance.desktop;\n[Added Associations]\nimage/png=other.desktop;\n";
        assert_eq!(merge_mimeapps(before, ENTRIES), after);
        assert_eq!(merge_mimeapps(after, ENTRIES), after);
    }
    #[test]
    fn missing_group_and_trailing_newline() {
        assert_eq!(
            merge_mimeapps("[Other]\nkey=value", ENTRIES),
            "[Other]\nkey=value\n[Default Applications]\nimage/png=glance.desktop;\nimage/jpeg=glance.desktop;\n"
        );
        assert_eq!(
            merge_mimeapps("[Default Applications]\nimage/png=old", &["image/png"]),
            "[Default Applications]\nimage/png=glance.desktop;\n"
        );
        assert_eq!(
            merge_mimeapps("[Default Applications]\n# tail", &["image/png"]),
            "[Default Applications]\n# tail\nimage/png=glance.desktop;\n"
        );
    }
}
