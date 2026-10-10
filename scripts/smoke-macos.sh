#!/bin/sh
# Foreground bounded smoke; launch/kill only the process started by this script.
set -eu
cd "$(dirname "$0")/.."
image=${1:?Usage: sh scripts/smoke-macos.sh /absolute/path/to/image}
case "$image" in /*) ;; *) echo 'IMAGE must be an absolute path.' >&2; exit 1 ;; esac
[ -f "$image" ] || { echo "Image not found: $image" >&2; exit 1; }
version=$(awk '/^\[package\]$/ { p=1; next } /^\[/ { p=0 } p && /^version = / { gsub(/"/, "", $3); print $3; exit }' Cargo.toml)
out=${MACOS_OUT:-"$PWD/target/macos-artifacts"}
name="Glance-$version-$(uname -m)"
work=$(mktemp -d /tmp/glance-macos-smoke.XXXXXX)
pid=
mounted=no
mkdir "$work/empty-loader-path"
avif_image=${AVIF_IMAGE:-"$PWD/assets/test/web/alpha12.avif"}
cleanup() {
    if [ -n "$pid" ]; then kill "$pid" 2>/dev/null || :; wait "$pid" 2>/dev/null || :; fi
    if [ "$mounted" = yes ]; then hdiutil detach "$work/mount" || return 1; fi
    # Keep complete launch logs for review; the mountpoint is empty after detach.
    echo "Smoke evidence: $work"
}
trap cleanup EXIT HUP INT TERM
sanitize_loader() {
    unset DYLD_INSERT_LIBRARIES DYLD_FRAMEWORK_PATH DYLD_FALLBACK_FRAMEWORK_PATH
    unset DYLD_VERSIONED_LIBRARY_PATH DYLD_VERSIONED_FRAMEWORK_PATH DYLD_ROOT_PATH
    export DYLD_LIBRARY_PATH="$work/empty-loader-path"
    export DYLD_FALLBACK_LIBRARY_PATH="$work/empty-loader-path"
    export DYLD_PRINT_LIBRARIES=1
}
check_avif() {
    binary=$1; label=$2
    [ "${SKIP_AVIF:-0}" != 1 ] || return 0
    (sanitize_loader; GLANCE_SMOKE_IMAGE=1 "$binary" "$avif_image") \
        > "$work/$label-avif.log" 2> "$work/$label-dyld.log"
    grep -E '^decoded [0-9]+x[0-9]+' "$work/$label-avif.log"
    frameworks="$(dirname "$binary")/../Frameworks"
    frameworks=$(cd "$frameworks" && pwd -P)
    for lib in libavif.16.dylib libdav1d.7.dylib; do
        grep -F "$frameworks/$lib" "$work/$label-dyld.log"
    done
    if grep -E '/opt/homebrew|/usr/local|/Cellar/' "$work/$label-dyld.log"; then
        echo "Package-manager code loaded; see $work/$label-dyld.log" >&2; return 1
    fi
}
launch() {
    binary=$1; label=$2
    check_avif "$binary" "$label"
    (sanitize_loader; exec "$binary" "$image") > "$work/$label.log" 2>&1 &
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
codesign --verify --deep --strict --verbose=2 "$out/Glance.app"
codesign -dv "$out/Glance.app"
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
codesign --verify --deep --strict --verbose=2 "$work/zip/Glance.app"
codesign -dv "$work/zip/Glance.app"
for file in MacOS/glance Resources/glance.icns Info.plist; do
    cmp "$out/Glance.app/Contents/$file" "$work/zip/Glance.app/Contents/$file"
done
if [ "${SKIP_NOTICES:-0}" != 1 ]; then
    diff -r "$out/Glance.app/Contents/Resources/licenses" "$work/zip/Glance.app/Contents/Resources/licenses"
fi
launch "$work/zip/Glance.app/Contents/MacOS/glance" zip
launch "$out/Glance.app/Contents/MacOS/glance" direct
mkdir "$work/mount"
hdiutil attach -readonly -nobrowse -mountpoint "$work/mount" "$out/$name.dmg" > "$work/mount.log"
mounted=yes
codesign --verify --deep --strict --verbose=2 "$work/mount/Glance.app"
codesign -dv "$work/mount/Glance.app"
[ "$(readlink "$work/mount/Applications")" = /Applications ]
if [ "${SKIP_NOTICES:-0}" != 1 ]; then
    diff -r "$out/Glance.app/Contents/Resources/licenses" "$work/mount/Glance.app/Contents/Resources/licenses"
fi
launch "$work/mount/Glance.app/Contents/MacOS/glance" dmg
hdiutil detach "$work/mount" > "$work/unmount.log"
mounted=no
rmdir "$work/mount"
# Extracted zip files are scratch created by this run, never user inputs.
rm -rf "$work/zip"
echo 'Zip, direct app and mounted DMG display smoke passed.'
