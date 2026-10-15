#!/usr/bin/env bash
# Run only the preserved synthetic benchmark, never the diagnostic GUI.
set -euo pipefail
fail() { printf '%s\n' "$*" >&2; exit 1; }
[[ $# == 2 ]] || fail "Usage: bash $0 RECONSTRUCTED_CHECKOUT NEW_OUTPUT_DIRECTORY"
package=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
source=$(cd -- "$1" && pwd -P)
base=a975449e4dd138ea4be84946b443f664a5d6cf59
[[ $(git -C "$source" rev-parse --show-toplevel) == "$source" ]] || fail "Use the checkout root"
[[ $(git -C "$source" rev-parse HEAD) == "$base" ]] || fail "Wrong base; expected $base"
if git -C "$source" symbolic-ref -q HEAD >/dev/null; then
    fail "Use a detached historical checkout, not a working branch"
fi
[[ ! -L "$source/target" ]] || fail "The checkout must have its own target directory"
if command -v sha256sum >/dev/null 2>&1; then
    sha=(sha256sum)
else
    command -v shasum >/dev/null 2>&1 || fail "Need sha256sum or shasum"
    sha=(shasum -a 256)
fi
parent=$(cd -- "$(dirname -- "$2")" && pwd -P)
out="$parent/$(basename -- "$2")"
case "$out" in "$source"|"$source"/*) fail "Keep output outside the reconstructed checkout" ;; esac
mkdir -- "$out" || fail "Output must be a NEW directory with an existing parent; nothing overwritten"
printf 'Full logs: %s\n' "$out"
trap 'printf "Run incomplete; retained logs: %s\n" "$out" >&2' ERR
cd -- "$source"
{
    "${sha[@]}" -c "$package/source.sha256"
    excludes=()
    while read -r _hash path; do
        excludes+=(":(exclude)$path")
    done < "$package/source.sha256"
    git diff --exit-code HEAD -- . "${excludes[@]}"
    while IFS= read -r path; do
        case "$path" in
            src/profile.rs|src/tiles/comparison.rs) ;;
            *) fail "Unexpected untracked file: $path" ;;
        esac
    done < <(git ls-files --others --exclude-standard)
    git apply --reverse --check "$package/overlay.patch"
} > "$out/verification.log" 2>&1
unset GLANCE_PROFILE_SECONDS GLANCE_PROFILE_DETAILS GLANCE_PROFILE_TIMERS GLANCE_PROFILE_TEST_MODE GLANCE_GUTTER_KERNEL
{
    date -u
    uname -sm
    rustc --version
    cargo --version
    git rev-parse HEAD
    "${sha[@]}" "$package/overlay.patch" "$package/source.sha256"
} > "$out/context.log" 2>&1
cargo test --release --locked --bin glance --target-dir "$source/target" --no-run > "$out/build.log" 2>&1
cargo test --release --locked --bin glance --target-dir "$source/target" \
    tiles::comparison::benchmark_gutters -- --exact --ignored --nocapture --test-threads=1 \
    > "$out/benchmark.log" 2>&1
printf 'Completed synthetic benchmark; see %s/benchmark.log\n' "$out"
