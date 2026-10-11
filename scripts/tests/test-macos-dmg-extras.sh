#!/bin/sh
# Hermetic DMG helper checks: no real app installation or desktop actions.
set -eu
cd "$(dirname "$0")/../.."
[ "$(uname -s)" = Darwin ] || { echo 'SKIP: macOS DMG helper checks require macOS.'; exit 0; }
work=$(mktemp -d /tmp/glance-dmg-extras-tests.XXXXXX)
trap 'rm -rf "$work"' EXIT HUP INT TERM
work=$(cd "$work" && pwd -P)
image="$work/image with spaces"
prefix="$work/install prefix"
home="$work/fake home"
mkdir -p "$prefix/Glance.app" "$home/.Trash" "$work/bin" "$work/outside"
printf 'existing app\n' > "$prefix/Glance.app/keep"
printf 'trash sentinel\n' > "$home/.Trash/keep"
printf 'outside sentinel\n' > "$work/outside/keep"
# Reject bad versions before creating any output (no app or network required).
for bad in '' 1 1.2 1.2.3.4 v1.2.3 1.2.3-rc1 1..3 1.2.x '1.2.3 trailing' '1.2.3
4.5.6'; do
    if sh scripts/macos-dmg-extras.sh "$work/rejected" "$bad" > "$work/version-error.log" 2>&1; then
        echo 'Unexpected success for malformed release version.' >&2; exit 1
    fi
    grep -F 'DMG extras require a release version (x.y.z) as the second argument.' "$work/version-error.log"
    [ ! -e "$work/rejected" ]
done
if sh scripts/macos-dmg-extras.sh "$work/rejected" > "$work/version-error.log" 2>&1; then
    echo 'Unexpected success without a release version.' >&2; exit 1
fi
grep -F 'DMG extras require a release version (x.y.z) as the second argument.' "$work/version-error.log"
[ ! -e "$work/rejected" ]
# Two versions ensure the generator does not just hardcode the current release.
for version in 9.8.7 12.34.56; do
    sh scripts/macos-dmg-extras.sh "$image" "$version"
    [ "$(grep -Eo '[0-9]+\.[0-9]+\.[0-9]+' "$image/Read Me.txt" | sort -u)" = "$version" ]
    grep -Fx "   asset names include the version. For the current release, $version:" "$image/Read Me.txt"
    for extension in dmg sha256; do
        grep -Fx "   curl -fL -O https://github.com/sdkks/glance/releases/download/v$version/Glance-$version-arm64.$extension" "$image/Read Me.txt"
    done
done
helper="$image/Install Glance.command"
[ "$(stat -f '%Lp' "$helper")" = 755 ]
[ -x "$helper" ]
sh -n "$helper"
bash -n "$helper"
grep -F 'action xattr -dr com.apple.quarantine "$destination"' "$helper"
grep -F 'action mv "$destination" "$trash/Glance.app"' "$helper"
grep -F 'action "$register" -f "$destination"' "$helper"
[ -f "$image/Read Me.txt" ]
grep -F 'self-signed' "$image/Read Me.txt"
grep -F 'not notarized' "$image/Read Me.txt"
grep -F 'Open Anyway' "$image/Read Me.txt"
grep -F 'xattr -dr com.apple.quarantine /Applications/Glance.app' "$image/Read Me.txt"
grep -Fx '   bash "/Volumes/Glance <version>/Install Glance.command"' "$image/Read Me.txt"
grep -F 'has no paid Apple Developer ID' "$image/Read Me.txt"
grep -F 'for a quarantined download. Developer ID-signed, notarized builds avoid' "$image/Read Me.txt"
grep -F "gh release download --repo sdkks/glance --pattern 'Glance-*-arm64.dmg' --pattern 'Glance-*-arm64.sha256'" "$image/Read Me.txt"
grep -Fx '   hdiutil attach Glance-*-arm64.dmg -mountpoint "$PWD/glance-volume"' "$image/Read Me.txt"
grep -Fx '   hdiutil detach "$PWD/glance-volume"' "$image/Read Me.txt"
grep -F "grep -E 'Glance-.*-arm64\.dmg\$' Glance-*-arm64.sha256 | shasum -a 256 -c -" "$image/Read Me.txt"
grep -F 'Continue only if the checksum reports OK. The .sha256 file lists both' "$image/Read Me.txt"
grep -F 'the zip and DMG; checking the whole file needs both downloads present.' "$image/Read Me.txt"
if grep -Ei 'homebrew|cask|package.manager' "$image/Read Me.txt"; then
    echo 'Read Me must not advertise an unavailable package-manager release.' >&2; exit 1
fi
grep -F 'Double-clicking it is blocked while the file is quarantined.' "$image/Read Me.txt"
# Keep the actionable manual routes ahead of the helper.
routes=$(grep -E '^[1-4]\. ' "$image/Read Me.txt")
[ "$routes" = '1. Terminal: drag Glance.app to Applications, then run:
2. CLI re-download: get a clean, non-quarantined copy with gh (GitHub CLI)
3. System Settings: drag Glance.app to Applications and try opening it.
4. Helper: run it from Terminal, replacing <version> with the DMG version:' ]
# All potentially mutating PATH commands fail loudly if dry-run executes them.
for tool in mkdir mv ditto xattr open pgrep pkill sleep; do
    printf '#!/bin/sh\nprintf "Unexpected action: %%s\\n" "$0" >&2\nexit 99\n' > "$work/bin/$tool"
    chmod +x "$work/bin/$tool"
done
printf '#!/bin/sh\n[ "$1" = -s ] && echo Darwin\n' > "$work/bin/uname"
chmod +x "$work/bin/uname"
expect_failure() {
    expected=$1; shift
    if HOME="$home" PATH="$work/bin:$PATH" "$@" > "$work/failure.log" 2>&1; then
        echo 'Unexpected helper success.' >&2; exit 1
    fi
    grep -F "$expected" "$work/failure.log"
}
expect_failure 'Glance.app is missing beside this helper:' bash "$helper"
mkdir -p "$image/Glance.app/Contents"
cat > "$image/Glance.app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>CFBundleShortVersionString</key><string>9.8.7</string></dict></plist>
PLIST
snapshot() {
    find "$image" "$prefix" "$home" "$work/outside" -print
    find "$image" "$prefix" "$home" "$work/outside" -type f -exec cksum {} +
}
snapshot > "$work/before"
HOME="$home" PATH="$work/bin:$PATH" PREFIX="$prefix" DRY_RUN=1 bash "$helper" > "$work/dry-run.log" 2>&1
snapshot > "$work/after"
cmp "$work/before" "$work/after"
grep -F "$prefix/Glance.app" "$work/dry-run.log"
grep -Fx 'Dry run: existing app would be moved to a new unique directory inside your Trash.' "$work/dry-run.log"
if grep -F 'Previous app preserved' "$work/dry-run.log"; then exit 1; fi
grep -F 'verify installed Info.plist version matches 9.8.7' "$work/dry-run.log"
if grep -F 'Unexpected action:' "$work/dry-run.log"; then exit 1; fi
# Missing destination is not created by preview either.
HOME="$home" PATH="$work/bin:$PATH" PREFIX="$work/new prefix" DRY_RUN=1 bash "$helper" > "$work/new-prefix.log" 2>&1
[ ! -e "$work/new prefix" ]
PREFIX=relative DRY_RUN=1 expect_failure 'PREFIX must be absolute.' bash "$helper"
PREFIX="$image" DRY_RUN=1 expect_failure 'install destination is the source disk image' bash "$helper"
DRY_RUN=bad expect_failure 'DRY_RUN must be 0 or 1.' bash "$helper"
ln -s "$prefix" "$work/linked prefix"
PREFIX="$work/linked prefix" DRY_RUN=1 expect_failure 'Refusing a symlink install directory' bash "$helper"
echo 'DMG extras, missing-source refusal, dry-run isolation and prefix checks passed.'
