#!/bin/sh
# Hermetic decision/safety checks: never build native code or use real build/.
set -eu
cd "$(dirname "$0")/../.."
[ "$(uname -s)" = Darwin ] || { echo 'SKIP: AVIF stage refresh checks require macOS.'; exit 0; }
work=$(mktemp -d /tmp/glance-avif-refresh-tests.XXXXXX)
trap 'rm -rf "$work"' EXIT HUP INT TERM
cp scripts/prepare-avif-stage.sh "$work/prepare.sh"
mkdir -p "$work/src" "$work/assets/icons/glance.iconset" "$work/build"
for input in src/main.rs Cargo.toml Cargo.lock build.rs assets/icons/glance.iconset/icon_256x256.png; do
    printf 'fixture\n' > "$work/$input"
    touch -t 202001010000 "$work/$input"
done
touch -t 202001010000 "$work/src"
cat > "$work/make" <<'STUB'
#!/bin/sh
set -eu
[ "$MACOSX_DEPLOYMENT_TARGET" = 11.0 ]
[ "$1" = bundle-avif ]
stage=${2#AVIF_STAGE=}
[ ! -e "$stage" ] && [ ! -L "$stage" ]
mkdir -p "$stage/Contents/MacOS"
printf 'rebuilt\n' > "$stage/Contents/MacOS/glance"
printf '%s\n' "$stage" >> calls
STUB
chmod +x "$work/make"
cd "$work"
make_stage() {
    mkdir -p "$1/Contents/MacOS"
    printf 'original\n' > "$1/Contents/MacOS/glance"
    touch -t 202001020000 "$1/Contents/MacOS/glance"
}
prepare() { sh ./prepare.sh "$1" "$work/make"; }
refused() {
    if prepare "$1" > refusal.log 2>&1; then
        echo "Unexpected refresh success: $1" >&2; exit 1
    fi
    grep -F 'Remove/refresh this path manually.' refusal.log
}
# Each guard input independently triggers refresh.
for input in src/main.rs Cargo.toml Cargo.lock build.rs assets/icons/glance.iconset/icon_256x256.png; do
    make_stage 'build/native stage'
    touch -t 202001030000 "$input"
    prepare 'build/native stage' > refresh.log
    grep -F 'Refreshing stale AVIF stage: build/native stage' refresh.log
    grep -Fx rebuilt 'build/native stage/Contents/MacOS/glance'
    touch -t 202001010000 "$input"
done
make_stage build/fresh
cp calls before-calls
prepare build/fresh
grep -Fx original build/fresh/Contents/MacOS/glance
cmp calls before-calls
prepare build/absent
grep -Fx rebuilt build/absent/Contents/MacOS/glance
# Stale invalid paths must keep their payload, including symlink ancestors.
for stage in outside "$work/outside" build build/ build/../outside build/./fresh; do
    make_stage "$stage"
    touch -t 201901010000 "$stage/Contents/MacOS/glance"
    refused "$stage"
    grep -Fx original "$stage/Contents/MacOS/glance"
done
ln -s ../outside build/link
refused build/link
test -L build/link
mkdir -p outside/nested/Contents/MacOS
printf 'original\n' > outside/nested/Contents/MacOS/glance
touch -t 201901010000 outside/nested/Contents/MacOS/glance
refused build/link/nested
grep -Fx original outside/nested/Contents/MacOS/glance
mkdir -p build/markerless
printf 'keep\n' > build/markerless/sentinel
refused build/markerless
grep -Fx keep build/markerless/sentinel
# Missing inputs fail closed, before deletion or rebuild.
make_stage build/missing-input
mv build.rs build.rs.missing
if prepare build/missing-input > missing.log 2>&1; then exit 1; fi
grep -F build.rs missing.log
grep -Fx original build/missing-input/Contents/MacOS/glance
echo 'AVIF stale/absent refresh, fresh reuse, unsafe-path preservation and missing-input checks passed.'
