#!/bin/sh
# Focused failure-path checks; real successful artifacts are covered by macos-smoke.
set -eu
cd "$(dirname "$0")/../.."
[ "$(uname -s)" = Darwin ] || { echo 'SKIP: macOS packaging checks require macOS.'; exit 0; }
work=$(mktemp -d /tmp/glance-macos-input-tests.XXXXXX)
trap 'rm -rf "$work"' EXIT HUP INT TERM
expect_failure() {
    expected=$1; shift
    if "$@" > "$work/failure.log" 2>&1; then
        echo 'Unexpected packaging success.' >&2; exit 1
    fi
    grep -F "$expected" "$work/failure.log"
}
expect_failure 'Missing AVIF stage input:' sh scripts/package-macos.sh "$work/missing"
stage="$work/native stage"
mkdir -p "$stage/Contents/MacOS" "$stage/Contents/Frameworks" "$stage/Contents/Resources/licenses/native"
# Input rejection must precede any Mach-O/signing/archive work.
for file in Contents/MacOS/glance Contents/Frameworks/libavif.16.dylib Contents/Frameworks/libdav1d.7.dylib \
    Contents/Resources/licenses/native/libavif-1.4.2-LICENSE Contents/Resources/licenses/native/dav1d-1.5.4-COPYING; do
    printf 'fixture\n' > "$stage/$file"
done
NOTICES_STAGE="$work/missing-notices" expect_failure 'Missing notices:' sh scripts/package-macos.sh "$stage"
mkdir -p "$work/notices/native"
printf 'fixture\n' > "$work/notices/LICENSE"
printf 'fixture\n' > "$work/notices/THIRD-PARTY-NOTICES.md"
printf 'conflict\n' > "$work/notices/native/libavif-1.4.2-LICENSE"
NOTICES_STAGE="$work/notices" expect_failure 'Conflicting native notice:' sh scripts/package-macos.sh "$stage"
echo 'Missing stage, missing notices and conflicting native notice checks passed.'
