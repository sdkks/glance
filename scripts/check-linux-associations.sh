#!/bin/sh
# Disposable Linux desktop association/dispatch check; no host installation.
set -eu
cd "$(dirname "$0")/.."
if ! command -v podman >/dev/null 2>&1; then
    printf '%s\n' 'SKIP: Linux association check requires podman and network.'
    exit 0
fi

# Stream both the committed desktop entry and the script through stdin: host
# bind mounts are not portable to a podman VM (notably on macOS).
{
    printf "desktop_base64='"
    base64 < resources/glance.desktop | tr -d '\n'
    printf "'\n"
    cat <<'LINUX_CHECK'
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends xdg-utils desktop-file-utils libglib2.0-bin
export HOME=/tmp/glance-home
export XDG_CONFIG_HOME="$HOME/.config"
export XDG_DATA_HOME="$HOME/.local/share"
export PATH="$HOME/bin:$PATH"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME/applications" "$HOME/bin"
desktop="$XDG_DATA_HOME/applications/glance.desktop"
printf '%s' "$desktop_base64" | base64 -d > "$desktop"
desktop-file-validate "$desktop"
# Exec=glance %f and TryExec stay unchanged; PATH resolves our logging shim.
cat > "$HOME/bin/glance" <<'SHIM'
#!/bin/sh
printf '%s\n' "$#" "$@" > /tmp/glance-launch-args
SHIM
chmod +x "$HOME/bin/glance"
update-desktop-database "$XDG_DATA_HOME/applications"
xdg-mime default glance.desktop image/png image/gif
mimeapps="$XDG_CONFIG_HOME/mimeapps.list"
test -f "$mimeapps"
grep -Fx '[Default Applications]' "$mimeapps"
for mime in image/png image/gif; do
    grep -Fx "$mime=glance.desktop" "$mimeapps" || grep -Fx "$mime=glance.desktop;" "$mimeapps"
    actual=$(xdg-mime query default "$mime")
    printf 'xdg-mime query default %s: %s\n' "$mime" "$actual"
    test "$actual" = glance.desktop
    actual=$(LC_ALL=C.UTF-8 gio mime "$mime")
    printf '%s\n' "$actual"
    printf '%s\n' "$actual" | grep -Fx "Default application for “$mime”: glance.desktop"
done
# A space in the path checks that %f delivers one argument, not shell words.
touch '/tmp/glance test.png'
gio launch "$desktop" '/tmp/glance test.png'
# gio launches asynchronously; wait briefly inside the disposable container.
for attempt in $(seq 1 50); do
    test ! -f /tmp/glance-launch-args || break
    sleep 0.1
done
printf '1\n/tmp/glance test.png\n' > /tmp/glance-expected-args
cmp /tmp/glance-expected-args /tmp/glance-launch-args
printf '%s\n' 'gio launch argv:'
cat /tmp/glance-launch-args
printf '%s\n' 'PASS: Linux MIME defaults, association readback, and desktop Exec dispatch.'
LINUX_CHECK
} | podman run --rm -i ubuntu:24.04 bash -s

# An optional native-host decode check exercises the real argv path without a
# display. Never try to run a Linux AppImage on a macOS host.
binary=
for candidate in target/release/glance target/debug/glance; do
    if [ -x "$candidate" ]; then binary="$PWD/$candidate"; break; fi
done
if [ -z "$binary" ] && [ "$(uname -s)" = Linux ]; then
    for candidate in build/Glance-*-"$(uname -m)".AppImage; do
        if [ -x "$candidate" ]; then binary="$PWD/$candidate"; break; fi
    done
fi
if [ -n "$binary" ]; then
    scratch=$(mktemp -d /tmp/glance-uri-smoke.XXXXXX)
    trap 'rm -f "$scratch/test.png"; rmdir "$scratch"' EXIT HUP INT TERM
    cp assets/icons/hicolor/256x256/apps/glance.png "$scratch/test.png"
    GLANCE_SMOKE_IMAGE=1 APPIMAGE_EXTRACT_AND_RUN=1 "$binary" "file://$scratch/test.png"
    printf '%s\n' 'PASS: real binary decodes a file:// argument through GLANCE_SMOKE_IMAGE=1.'
else
    printf '%s\n' 'SKIP: real file:// decode (no built native binary or AppImage in this checkout).'
fi
