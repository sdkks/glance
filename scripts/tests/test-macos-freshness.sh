#!/bin/sh
# Hermetic freshness checks: reach the notices boundary, never native tooling.
set -eu
cd "$(dirname "$0")/../.."
[ "$(uname -s)" = Darwin ] || { echo 'SKIP: macOS freshness checks require macOS.'; exit 0; }
work=$(mktemp -d /tmp/glance-macos-freshness-tests.XXXXXX)
trap 'rm -rf "$work"' EXIT HUP INT TERM
mkdir -p "$work/scripts" "$work/src" "$work/bin" "$work/assets/icons/glance.iconset"
cp scripts/package-macos.sh "$work/scripts/"
# Stub platform queries so fixtures never need native tools.
printf '#!/bin/sh\ncase "$1" in -s) echo Darwin ;; -m) echo arm64 ;; *) exit 1 ;; esac\n' > "$work/bin/uname"
chmod +x "$work/bin/uname"
printf '[package]\nversion = "0.1.0"\n' > "$work/Cargo.toml"
for input in src/main.rs Cargo.lock build.rs assets/icons/glance.iconset/icon_256x256.png; do
    printf 'fixture\n' > "$work/$input"
done
stage="$work/native stage"
mkdir -p "$stage/Contents/MacOS" "$stage/Contents/Frameworks" "$stage/Contents/Resources/licenses/native"
for file in Contents/MacOS/glance Contents/Frameworks/libavif.16.dylib Contents/Frameworks/libdav1d.7.dylib \
    Contents/Resources/licenses/native/libavif-1.4.2-LICENSE Contents/Resources/licenses/native/dav1d-1.5.4-COPYING; do
    printf 'fixture\n' > "$stage/$file"
done
binary="$stage/Contents/MacOS/glance"
reset_inputs() {
    find "$work/src" -exec touch -t 202001010000 {} +
    touch -t 202001010000 "$work/Cargo.toml" "$work/Cargo.lock" "$work/build.rs" \
        "$work/assets/icons/glance.iconset/icon_256x256.png"
    touch -t 202001020000 "$binary"
}
expect_failure() {
    if PATH="$work/bin:$PATH" NOTICES_STAGE="$work/missing-notices" SKIP_AVIF=0 SKIP_NOTICES=0 \
        sh "$work/scripts/package-macos.sh" "$stage" > "$work/failure.log" 2>&1; then
        echo 'Unexpected packaging success.' >&2; exit 1
    fi
}
for input in src/main.rs Cargo.toml Cargo.lock build.rs assets/icons/glance.iconset/icon_256x256.png; do
    reset_inputs
    touch -t 202001030000 "$work/$input"
    expect_failure
    grep -F 'Stale AVIF stage executable:' "$work/failure.log"
    grep -Fx "$input" "$work/failure.log"
    grep -Fx 'The default stage cannot be refreshed in place; the builder refuses to reuse a directory.' "$work/failure.log"
    grep -Fx 'Build a fresh stage: make bundle-avif AVIF_STAGE=<new-empty-path>' "$work/failure.log"
    grep -Fx 'Then package it: make macos-artifacts AVIF_STAGE=<new-empty-path>' "$work/failure.log"
done
reset_inputs
expect_failure
grep -F 'Missing notices:' "$work/failure.log"
if grep -F 'Stale AVIF stage executable:' "$work/failure.log"; then
    echo 'Fresh stage incorrectly rejected.' >&2; exit 1
fi
# Missing build inputs must fail closed too, rather than silently passing find.
mv "$work/build.rs" "$work/build.rs.missing"
expect_failure
grep -F 'build.rs' "$work/failure.log"
if grep -F 'Missing notices:' "$work/failure.log"; then
    echo 'Missing build input incorrectly accepted.' >&2; exit 1
fi
echo 'Stale source/manifest/lock/build/icon rejection, fresh-stage acceptance and missing-input checks passed.'
