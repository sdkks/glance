#!/bin/sh
# Generate only DMG extras; the app and zip payloads remain independent.
set -eu
# Usage: sh scripts/macos-dmg-extras.sh <target-directory> <x.y.z>
output=${1:?target directory required}
version=${2:-}
fail_version() {
    printf '%s\n' 'DMG extras require a release version (x.y.z) as the second argument.' >&2
    exit 1
}
case "$version" in ''|*[!0-9.]*) fail_version ;; esac
if ! printf '%s\n' "$version" | LC_ALL=C grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    fail_version
fi
mkdir -p "$output"
cat > "$output/Install Glance.command" <<'HELPER'
#!/bin/bash
# Finder blocks this helper while quarantined; run it from Terminal with bash.
# Preview without changes: PREFIX=/absolute/path DRY_RUN=1 bash ...
set -eu
fail() { printf 'Install failed: %s\n' "$*" >&2; exit 1; }
action() {
    printf ' %q' "$@"; printf '\n'
    if [ "$dry" != 1 ]; then "$@" || fail "Command failed; see above. Any previous app remains in $trash."; fi
}
dry=${DRY_RUN:-0}
case "$dry" in 0|1) ;; *) fail 'DRY_RUN must be 0 or 1.' ;; esac
[ "$(uname -s)" = Darwin ] || fail 'This installer requires macOS.'
source_dir=$(cd "$(dirname "$0")" && pwd -P)
source_app="$source_dir/Glance.app"
[ -d "$source_app" ] || fail "Glance.app is missing beside this helper: $source_app"
version=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$source_app/Contents/Info.plist") || fail 'Cannot read the source bundle version.'
[ -n "$version" ] || fail 'The source bundle version is empty.'
prefix=${PREFIX:-/Applications}
if [ -z "${PREFIX:-}" ] && [ ! -w /Applications ]; then
    [ -n "${HOME:-}" ] || fail 'HOME is unset; cannot use ~/Applications.'
    prefix="$HOME/Applications"
    printf '/Applications is not writable; installing in %s instead.\n' "$prefix"
fi
case "$prefix" in /*) ;; *) fail 'PREFIX must be absolute.' ;; esac
prefix=${prefix%/}
case "$prefix/" in /|*/../*|*/./*|*//*) fail 'Use a non-root PREFIX without dot components or repeated slashes.' ;; esac
[ ! -L "$prefix" ] || fail "Refusing a symlink install directory: $prefix"
if [ -d "$prefix" ]; then
    prefix=$(cd "$prefix" && pwd -P)
    [ "$prefix" != "$source_dir" ] || fail 'The install destination is the source disk image.'
    [ -w "$prefix" ] || fail "Install directory is not writable: $prefix"
fi
destination="$prefix/Glance.app"
[ ! -L "$destination" ] || fail "Refusing a symlink destination app: $destination"
register=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
[ -x "$register" ] || fail 'macOS application registration tool is missing.'
trash='the user Trash'
printf 'Installing Glance %s from %s to %s\n' "$version" "$source_app" "$destination"
# Dry-run must not send signals, create directories, strip flags or launch apps.
if [ "$dry" = 1 ]; then
    printf 'Dry run: quit any running Glance owned by this user.\n'
else
    if pgrep -u "$(id -u)" -x glance >/dev/null; then
        printf 'Quitting running Glance.\n'
        pkill -TERM -u "$(id -u)" -x glance || fail 'Could not quit Glance.'
        attempts=0
        while pgrep -u "$(id -u)" -x glance >/dev/null; do
            attempts=$((attempts + 1))
            [ "$attempts" -lt 10 ] || fail 'Glance is still running. Quit it and try again.'
            sleep 1
        done
    fi
fi
action mkdir -p "$prefix"
if [ -e "$destination" ]; then
    [ -n "${HOME:-}" ] || fail 'HOME is unset; cannot preserve the existing app in Trash.'
    if [ "$dry" = 1 ]; then
        trash="$HOME/.Trash/Glance-install.XXXXXX"
        printf 'Dry run: create a unique directory in %s/.Trash.\n' "$HOME"
    else
        [ -d "$HOME/.Trash" ] && [ -w "$HOME/.Trash" ] || fail 'User Trash is unavailable or not writable; existing app has not been moved.'
        trash=$(mktemp -d "$HOME/.Trash/Glance-install.XXXXXX") || fail 'Cannot create a safe location in Trash.'
    fi
    action mv "$destination" "$trash/Glance.app"
    if [ "$dry" = 1 ]; then
        printf 'Dry run: existing app would be moved to a new unique directory inside your Trash.\n'
    else
        printf 'Previous app preserved in %s/Glance.app\n' "$trash"
    fi
fi
action ditto "$source_app" "$destination"
action xattr -dr com.apple.quarantine "$destination"
action "$register" -f "$destination"
if [ "$dry" = 1 ]; then
    printf 'Dry run: verify installed Info.plist version matches %s.\n' "$version"
else
    installed=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$destination/Contents/Info.plist") || fail 'Cannot read the installed bundle version.'
    [ "$installed" = "$version" ] || fail "Installed version mismatch: expected $version, found $installed."
    printf 'Verified installed Glance %s at %s\n' "$installed" "$destination"
fi
action open "$destination"
printf 'Done.\n'
HELPER
chmod 755 "$output/Install Glance.command"
cat > "$output/Read Me.txt" <<README
Installing Glance on macOS

Glance is self-signed (ad-hoc signed) and not notarized because this project
has no paid Apple Developer ID. There is no double-click first-launch bypass
for a quarantined download. Developer ID-signed, notarized builds avoid
this friction; CLI downloads are not quarantined either. Only bypass the
block if you trust the download and have checked its source and checksum.

Choose one of these routes:

1. Terminal: drag Glance.app to Applications, then run:
   xattr -dr com.apple.quarantine /Applications/Glance.app
   Open the installed app normally afterwards. Use the installed path if
   you chose a different directory.
2. CLI re-download: get a clean, non-quarantined copy with gh (GitHub CLI)
   or curl. No xattr command or System Settings workaround is needed.
   With gh installed (no account or sign-in needed), run in a fresh directory:
   gh release download --repo sdkks/glance --pattern 'Glance-*-arm64.dmg' --pattern 'Glance-*-arm64.sha256'
   grep -E 'Glance-.*-arm64\.dmg$' Glance-*-arm64.sha256 | shasum -a 256 -c -
   Continue only if the checksum reports OK. The .sha256 file lists both
   the zip and DMG; checking the whole file needs both downloads present.
   The grep command selects only the DMG line.

   For curl, copy asset URLs from https://github.com/sdkks/glance/releases;
   asset names include the version. For the current release, $version:
   curl -fL -O https://github.com/sdkks/glance/releases/download/v$version/Glance-$version-arm64.dmg
   curl -fL -O https://github.com/sdkks/glance/releases/download/v$version/Glance-$version-arm64.sha256
   grep -E 'Glance-.*-arm64\.dmg$' Glance-*-arm64.sha256 | shasum -a 256 -c -
   Continue only if the checksum reports OK. Eject the old DMG, quit Glance,
   then install the new copy (requires write access to /Applications):
   mkdir glance-volume
   hdiutil attach Glance-*-arm64.dmg -mountpoint "\$PWD/glance-volume"
   ditto glance-volume/Glance.app /Applications/Glance.app
   hdiutil detach "\$PWD/glance-volume"
   open /Applications/Glance.app
3. System Settings: drag Glance.app to Applications and try opening it.
   Then go to System Settings -> Privacy & Security -> "Open Anyway"
   and confirm the prompt.
4. Helper: run it from Terminal, replacing <version> with the DMG version:
   bash "/Volumes/Glance <version>/Install Glance.command"
   Double-clicking it is blocked while the file is quarantined. The helper
   quits Glance, preserves an existing copy in your Trash, installs in
   /Applications (or ~/Applications if /Applications is not writable),
   removes the quarantine flag, registers the app and launches it.

Do not run the app from this disk image. Eject the image after installation.

Preview the helper from Terminal without making changes:
DRY_RUN=1 bash "/Volumes/Glance <version>/Install Glance.command"
To choose another install directory, set PREFIX to an absolute path.
README
