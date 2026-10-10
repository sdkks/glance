#!/bin/sh
# Prefix installs own only the enumerated files, never shared directories.
set -eu
mode=${1:?install or uninstall required}
prefix=${2:?absolute prefix required}
staging=${3-}
dry=${4:-0}
confirm=${5:-0}
case "$mode" in install|uninstall) ;; *) exit 2 ;; esac
case "$prefix" in /*) ;; *) echo 'PREFIX must be absolute' >&2; exit 1 ;; esac
case "$staging" in ''|/*) ;; *) echo 'DESTDIR must be absolute or empty' >&2; exit 1 ;; esac
# Reject ambiguous traversal, including bypasses of the system-prefix guard.
for base in "$prefix" "$staging"; do
    case "$base/" in */../*|*/./*|*//*) echo 'Use canonical paths without dot components or repeated slashes' >&2; exit 1 ;; esac
done
prefix=${prefix%/}
staging=${staging%/}
[ -n "$prefix" ] || { echo 'PREFIX cannot be /' >&2; exit 1; }
case "$prefix" in /usr|/usr/*|/opt|/opt/*|/bin|/sbin|/lib|/lib/*|/etc|/etc/*)
    if [ -z "$staging" ] && [ "$confirm" != 1 ]; then
        echo 'System prefix refused: explicitly set SUDO=1 to confirm (sudo is never invoked).' >&2
        exit 1
    fi ;;
esac
root=$staging$prefix
manifest=$root/share/glance/install-manifest
sizes='16 24 32 48 64 128 256'
# Check the entire destination ancestry before any build or filesystem writes.
# Checking only root and its children misses symlinks in DESTDIR or PREFIX.
ancestor=$root
while [ "$ancestor" != / ]; do
    [ ! -L "$ancestor" ] || {
        echo "Refusing symlink destination ancestor: $ancestor; no files changed" >&2; exit 1
    }
    ancestor=${ancestor%/*}
    [ -n "$ancestor" ] || ancestor=/
done
# Refuse redirected destination directories, including dangling symlinks.
for dir in "$root" "$root/bin" "$root/share" "$root/share/applications" \
    "$root/share/glance" "$root/share/icons" "$root/share/icons/hicolor"; do
    [ ! -L "$dir" ] || { echo "Refusing symlink directory: $dir" >&2; exit 1; }
done
for size in $sizes; do
    for dir in "$root/share/icons/hicolor/${size}x${size}" "$root/share/icons/hicolor/${size}x${size}/apps"; do
        [ ! -L "$dir" ] || { echo "Refusing symlink directory: $dir" >&2; exit 1; }
    done
done
[ ! -L "$manifest" ] || { echo 'Refusing symlink manifest' >&2; exit 1; }
paths() {
    printf '%s\n' bin/glance share/applications/glance.desktop
    for size in $sizes; do printf 'share/icons/hicolor/%sx%s/apps/glance.png\n' "$size" "$size"; done
}
# Separate printing from execution so dry-run never even creates a build tree.
action() {
    printf '%s' "$1"
    command=$1
    shift
    for arg do printf " '%s'" "$arg"; done
    printf '\n'
    if [ "$dry" != 1 ]; then "$command" "$@"; fi
}
validate_manifest() {
    # Fixed order and fixed paths prevent a manifest from naming unrelated files.
    expected=$(paths)
    actual=$(while read -r sum bytes path; do printf '%s\n' "$path"; done < "$manifest")
    [ "$actual" = "$expected" ] || { echo 'Invalid manifest; no files changed' >&2; exit 1; }
    while read -r sum bytes path; do
        file=$root/$path
        if [ ! -f "$file" ] || [ -L "$file" ]; then
            echo "Missing or replaced installed file: $file; no files changed" >&2; exit 1
        fi
        current=$(cksum < "$file")
        [ "$current" = "$sum $bytes" ] || {
            echo "Modified installed file: $file; no files changed" >&2; exit 1
        }
    done < "$manifest"
}
refresh() {
    if [ -n "$staging" ] || [ "$prefix" != "${HOME:-}/.local" ]; then
        echo 'Skipping cache refresh outside the unstaged user prefix'
        return
    fi
    if command -v update-desktop-database >/dev/null 2>&1; then
        action update-desktop-database "$root/share/applications" || echo 'Warning: desktop cache refresh failed' >&2
    fi
    if [ -d "$root/share/mime" ] && [ ! -L "$root/share/mime" ] && command -v update-mime-database >/dev/null 2>&1; then
        action update-mime-database "$root/share/mime" || echo 'Warning: MIME cache refresh failed' >&2
    fi
}
if [ "$mode" = uninstall ]; then
    if [ ! -f "$manifest" ]; then
        echo "Warning: installed manifest missing at $manifest; nothing removed" >&2
        exit 0
    fi
    validate_manifest
    while read -r sum bytes path; do action rm -f "$root/$path"; done < "$manifest"
    action rm -f "$manifest"
    refresh
    exit 0
fi
if [ -e "$manifest" ]; then
    validate_manifest
else
    for path in $(paths); do
        if [ -e "$root/$path" ] || [ -L "$root/$path" ]; then
            echo "Refusing to overwrite unowned file: $root/$path" >&2; exit 1
        fi
    done
fi
action cargo build --release --locked
for path in $(paths); do
    case "$path" in
        bin/glance) source=target/release/glance; permissions=755 ;;
        share/applications/*) source=resources/glance.desktop; permissions=644 ;;
        *) source=assets/icons/hicolor/${path#share/icons/hicolor/}; permissions=644 ;;
    esac
    action mkdir -p "$root/${path%/*}"
    action install -m "$permissions" "$source" "$root/$path"
done
action mkdir -p "$root/share/glance"
printf 'write checksums to %s\n' "$manifest"
if [ "$dry" != 1 ]; then
    # Complete manifest is written only after every payload was installed.
    for path in $(paths); do
        checksum=$(cksum < "$root/$path")
        printf '%s %s\n' "$checksum" "$path"
    done > "$manifest"
fi
refresh
