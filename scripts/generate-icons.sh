#!/bin/sh
# Rebuild derived icons without changing the committed cropped artwork/masters.
set -eu
cd "$(dirname "$0")/.."
for tool in sips iconutil; do
    command -v "$tool" >/dev/null 2>&1 || {
        printf 'Missing %s: make icons requires macOS system tools. See README.\n' "$tool" >&2
        exit 1
    }
done
scratch=$(mktemp -d "${TMPDIR:-/tmp}/glance-icons.XXXXXX")
trap 'rm -rf "$scratch"' EXIT HUP INT TERM
mkdir -p "$scratch/glance.iconset" "$scratch/hicolor"
mac_master=assets/icon/glance-icon-master.png
linux_master=assets/icon/glance-icon-linux-master.png

render() {
    sips -z "$2" "$2" "$1" --out "$3" >/dev/null
}

for size in 16 32 128 256 512; do
    render "$mac_master" "$size" "$scratch/glance.iconset/icon_${size}x${size}.png"
    render "$mac_master" "$((size * 2))" "$scratch/glance.iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$scratch/glance.iconset" -o "$scratch/glance.icns"
for size in 16 24 32 48 64 128 256; do
    mkdir -p "$scratch/hicolor/${size}x${size}/apps"
    render "$linux_master" "$size" "$scratch/hicolor/${size}x${size}/apps/glance.png"
done

# Stage all generation first; retain existing files/mtimes when bytes match.
publish() {
    mkdir -p "$(dirname "$2")"
    if ! cmp -s "$1" "$2"; then
        cp "$1" "$2"
    fi
}
publish "$scratch/glance.icns" resources/glance.icns
for size in 16 32 128 256 512; do
    for suffix in '' '@2x'; do
        name="icon_${size}x${size}${suffix}.png"
        publish "$scratch/glance.iconset/$name" "assets/icons/glance.iconset/$name"
    done
done
for size in 16 24 32 48 64 128 256; do
    name="${size}x${size}/apps/glance.png"
    publish "$scratch/hicolor/$name" "assets/icons/hicolor/$name"
done
printf '%s\n' 'Generated resources/glance.icns, assets/icons/glance.iconset, assets/icons/hicolor.'
