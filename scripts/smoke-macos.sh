#!/bin/sh
# Foreground bounded smoke; launch/kill only the process started by this script.
set -eu
cd "$(dirname "$0")/.."
image=${1:?Usage: sh scripts/smoke-macos.sh /absolute/path/to/image}
case "$image" in /*) ;; *) echo 'IMAGE must be an absolute path.' >&2; exit 1 ;; esac
[ -f "$image" ] || { echo "Image not found: $image" >&2; exit 1; }
version=$(awk '/^\[package\]$/ { p=1; next } /^\[/ { p=0 } p && /^version = / { gsub(/"/, "", $3); print $3; exit }' Cargo.toml)
out="$PWD/target/macos-artifacts"
name="Glance-$version-$(uname -m)"
work=$(mktemp -d /tmp/glance-macos-smoke.XXXXXX)
pid=
mounted=no
cleanup() {
    if [ -n "$pid" ]; then kill "$pid" 2>/dev/null || :; wait "$pid" 2>/dev/null || :; fi
    if [ "$mounted" = yes ]; then hdiutil detach "$work/mount" || return 1; fi
    # Keep complete launch logs for review; the mountpoint is empty after detach.
    echo "Smoke evidence: $work"
}
trap cleanup EXIT HUP INT TERM
launch() {
    binary=$1; label=$2
    "$binary" "$image" > "$work/$label.log" 2>&1 &
    pid=$!
    attempt=0
    while [ "$attempt" -lt 30 ]; do
        if grep -F "Displayed $image (" "$work/$label.log"; then
            kill "$pid"; wait "$pid" 2>/dev/null || :; pid=
            return 0
        fi
        kill -0 "$pid" 2>/dev/null || { echo "App exited; see $work/$label.log" >&2; return 1; }
        attempt=$((attempt + 1))
        sleep 1
    done
    echo "No Displayed line after 30s; see $work/$label.log" >&2
    return 1
}
(cd "$out" && shasum -a 256 -c "$name.sha256")
plist="$out/Glance.app/Contents/Info.plist"
plutil -lint "$plist"
[ "$(plutil -extract CFBundleShortVersionString raw "$plist")" = "$version" ]
[ "$(plutil -extract CFBundleVersion raw "$plist")" = "$version" ]
[ "$(plutil -extract CFBundleExecutable raw "$plist")" = glance ]
[ "$(plutil -extract CFBundleIconFile raw "$plist")" = glance.icns ]
[ "$(plutil -extract LSMinimumSystemVersion raw "$plist")" = 11.0 ]
# Each of the 23 format groups must stay an alternate viewer, not a default.
index=0
while [ "$index" -lt 23 ]; do
    [ "$(plutil -extract "CFBundleDocumentTypes.$index.CFBundleTypeRole" raw "$plist")" = Viewer ]
    [ "$(plutil -extract "CFBundleDocumentTypes.$index.LSHandlerRank" raw "$plist")" = Alternate ]
    index=$((index + 1))
done
ditto -x -k "$out/$name.zip" "$work/zip"
for file in MacOS/glance Resources/glance.icns Info.plist; do
    cmp "$out/Glance.app/Contents/$file" "$work/zip/Glance.app/Contents/$file"
done
launch "$work/zip/Glance.app/Contents/MacOS/glance" zip
launch "$out/Glance.app/Contents/MacOS/glance" direct
mkdir "$work/mount"
hdiutil attach -readonly -nobrowse -mountpoint "$work/mount" "$out/$name.dmg" > "$work/mount.log"
mounted=yes
[ "$(readlink "$work/mount/Applications")" = /Applications ]
launch "$work/mount/Glance.app/Contents/MacOS/glance" dmg
hdiutil detach "$work/mount" > "$work/unmount.log"
mounted=no
rmdir "$work/mount"
# Extracted zip files are scratch created by this run, never user inputs.
rm -rf "$work/zip"
echo 'Zip, direct app and mounted DMG display smoke passed.'
