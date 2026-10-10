#!/bin/sh
# Native Linux build; unmodified, checksum-pinned decoder sources only.
set -eu
cd "$(dirname "$0")/.."
[ "$(uname -s)" = Linux ] || { echo 'Build AppImages on Linux (see README).' >&2; exit 1; }
case "$(uname -m)" in
    x86_64) arch=x86_64; tool_hash=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
        runtime_hash=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d ;;
    aarch64|arm64) arch=aarch64; tool_hash=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158
        runtime_hash=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444 ;;
    *) echo 'Supported build hosts: Linux x86_64 and aarch64.' >&2; exit 1 ;;
esac
for tool in cargo cmake meson ninja pkg-config curl sha256sum tar patchelf desktop-file-validate; do
    command -v "$tool" >/dev/null || { echo "Missing prerequisite: $tool" >&2; exit 1; }
done
# Persistent, architecture-separated build cache, not temporary directories.
# Serial invocation only; repeat builds refresh the same owned payload.
root="$PWD/build/appimage-$arch"
work="$root/native"
prefix="$work/prefix"
app="$root/Glance.AppDir"
libs="$app/usr/lib"
notices="$app/usr/share/glance/licenses/native"
mkdir -p "$work" "$libs" "$notices" "$app/usr/bin" "$app/usr/share/applications" "$app/usr/share/icons"
fetch() {
    curl --fail --location --retry 3 "$1" -o "$2"
    printf '%s  %s\n' "$3" "$2" | sha256sum -c -
}
avif_hash=2b645287340ba5a631d268b551dc2d72bd73ac33335962dd36dcdb6d8366921d
dav1d_hash=a1d5b63d2d38ec9bd03acf643caa51fa22edd1e89c5a109c4807717216bbec07
fetch https://code.videolan.org/videolan/dav1d/-/archive/1.5.4/dav1d-1.5.4.tar.gz "$work/dav1d.tar.gz" "$dav1d_hash"
fetch https://github.com/AOMediaCodec/libavif/archive/refs/tags/v1.4.2.tar.gz "$work/libavif.tar.gz" "$avif_hash"
tar -xzf "$work/dav1d.tar.gz" -C "$work"
tar -xzf "$work/libavif.tar.gz" -C "$work"
if [ ! -f "$work/dav1d-build/build.ninja" ]; then
    meson setup "$work/dav1d-build" "$work/dav1d-1.5.4" --prefix="$prefix" --libdir=lib \
        --buildtype=release --default-library=shared -Denable_tools=false -Denable_tests=false
fi
ninja -C "$work/dav1d-build"
ninja -C "$work/dav1d-build" install
export PKG_CONFIG_PATH="$prefix/lib/pkgconfig"
cmake -S "$work/libavif-1.4.2" -B "$work/avif-build" \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$prefix" -DCMAKE_INSTALL_LIBDIR=lib \
    '-DCMAKE_INSTALL_RPATH=$ORIGIN' \
    -DBUILD_SHARED_LIBS=ON -DAVIF_CODEC_DAV1D=SYSTEM \
    -DAVIF_CODEC_AOM=OFF -DAVIF_CODEC_AVM=OFF -DAVIF_CODEC_LIBGAV1=OFF \
    -DAVIF_CODEC_RAV1E=OFF -DAVIF_CODEC_SVT=OFF -DAVIF_LIBYUV=OFF \
    -DAVIF_LIBSHARPYUV=OFF -DAVIF_BUILD_APPS=OFF \
    -DAVIF_BUILD_EXAMPLES=OFF -DAVIF_BUILD_TESTS=OFF
cmake --build "$work/avif-build" --parallel "${JOBS:-2}"
cmake --install "$work/avif-build"
[ "$(pkg-config --modversion libavif)" = 1.4.2 ]
[ "$(pkg-config --modversion dav1d)" = 1.5.4 ]
# Separate Cargo artifacts from development builds and other architectures.
export CARGO_TARGET_DIR="$root/target"
cargo build --release --locked
cp "$CARGO_TARGET_DIR/release/glance" "$app/usr/bin/glance"
cp -L "$prefix/lib/libavif.so.16" "$libs/libavif.so.16"
cp -L "$prefix/lib/libdav1d.so.7" "$libs/libdav1d.so.7"
patchelf --set-rpath '$ORIGIN/../lib' "$app/usr/bin/glance"
for lib in "$libs"/*.so.*; do
    patchelf --set-rpath '$ORIGIN' "$lib"
    # Decoder closure fails closed: no AOM, encoders, vmaf, heif, external yuv.
    patchelf --print-needed "$lib" > "$root/needed-$(basename "$lib").txt"
    while IFS= read -r dep; do
        case "$dep" in
            libdav1d.so.7|libc.so.6|libm.so.6|libpthread.so.0|libdl.so.2|librt.so.1|ld-linux-*.so.*) ;;
            *) echo "Unapproved native dependency: $dep" >&2; exit 1 ;;
        esac
    done < "$root/needed-$(basename "$lib").txt"
done
patchelf --print-needed "$app/usr/bin/glance" | grep -Fx libavif.so.16
cp "$work/libavif-1.4.2/LICENSE" "$notices/libavif-1.4.2-LICENSE"
cp "$work/dav1d-1.5.4/COPYING" "$notices/dav1d-1.5.4-COPYING"
cp "$work/libavif-1.4.2/third_party/libyuv/AUTHORS" "$notices/embedded-libyuv-AUTHORS"
(cd "$work/libavif-1.4.2" && sha256sum third_party/libyuv/source/*.c) > "$notices/embedded-libyuv-source-sha256.txt"
printf 'libavif 1.4.2 source SHA-256 %s\ndav1d 1.5.4 source SHA-256 %s\nappimagetool 1.9.1 %s SHA-256 %s\n' \
    "$avif_hash" "$dav1d_hash" "$arch" "$tool_hash" > "$notices/provenance.txt"
cp LICENSE THIRD_PARTY.md "$app/usr/share/glance/licenses/"
# The sibling notices generator may arrive after this script; preserve native notices.
if [ -f scripts/generate-notices.py ]; then
    python3 scripts/generate-notices.py --native-notices "$notices" \
        --output "$app/usr/share/glance/notices"
else
    echo 'WARNING: complete Rust/font notices require the notices generator; see README.' >&2
fi
cp -R assets/icons/hicolor "$app/usr/share/icons/"
cat > "$app/usr/share/applications/glance.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=Glance
Comment=View images and browse their folder
Exec=glance %f
Icon=glance
Terminal=false
Categories=Graphics;Viewer;
MimeType=image/jpeg;image/png;image/tiff;image/bmp;image/x-icon;image/x-tga;image/x-portable-anymap;image/vnd.wap.wbmp;image/x-xbitmap;image/x-xpixmap;image/x-pcx;image/x-sgi;image/gif;image/webp;image/avif;image/jxl;image/svg+xml;image/svg+xml-compressed;image/vnd.adobe.photoshop;application/x-krita;image/openraster;image/icns;
DESKTOP
desktop-file-validate "$app/usr/share/applications/glance.desktop"
cp "$app/usr/share/applications/glance.desktop" "$app/glance.desktop"
cp "$app/usr/share/icons/hicolor/256x256/apps/glance.png" "$app/glance.png"
cat > "$app/AppRun" <<'APPRUN'
#!/bin/sh
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# RUNPATH resolves native libraries; this also covers dynamically loaded libraries.
export LD_LIBRARY_PATH="$here/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$here/usr/bin/glance" "$@"
APPRUN
chmod +x "$app/AppRun"
# Verify actual decoder loads from the payload, before creating the image.
LD_LIBRARY_PATH="$libs" ldd "$app/usr/bin/glance" > "$root/ldd.txt"
for name in libavif.so.16 libdav1d.so.7; do
    grep -F "$libs/$name" "$root/ldd.txt"
done
GLANCE_SMOKE_IMAGE=1 "$app/AppRun" "$PWD/assets/test/web/alpha8.avif" > "$root/smoke.txt"
grep -Fx 'decoded 5x3' "$root/smoke.txt"
fetch "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-$arch.AppImage" "$root/appimagetool.AppImage" "$tool_hash"
chmod +x "$root/appimagetool.AppImage"
fetch "https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-$arch" "$root/runtime" "$runtime_hash"
fetch https://github.com/AppImage/type2-runtime/archive/refs/tags/20251108.tar.gz \
    "$work/runtime-source.tar.gz" 4c4f6df4647c9f01f871d7edd3716d8aeeffda9d22ffbebe3fccd95a6ab52c95
tar -xzf "$work/runtime-source.tar.gz" -C "$work"
cp "$work/type2-runtime-20251108/LICENSE" "$app/usr/share/glance/licenses/AppImage-runtime-LICENSE"
printf 'type2-runtime 20251108 %s SHA-256 %s\nSource: https://github.com/AppImage/type2-runtime/tree/20251108\n' \
    "$arch" "$runtime_hash" > "$app/usr/share/glance/licenses/AppImage-runtime-provenance.txt"
version=$(awk '/^[ \t]*\[/ {package=($0 ~ /^[ \t]*\[package\][ \t]*(#.*)?$/)} package && /^[ \t]*version[ \t]*=/ {split($0,v,"\""); print v[2]}' Cargo.toml)
[ -n "$version" ] || { echo 'Missing Cargo package version.' >&2; exit 1; }
artifact="$PWD/build/Glance-$version-$arch.AppImage"
# Normalize payload timestamps; compiler/toolchain/OS still must be held fixed.
SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-0}
export SOURCE_DATE_EPOCH
find "$app" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
# No FUSE needed during build; do not download a mutable continuous runtime.
ARCH="$arch" "$root/appimagetool.AppImage" --appimage-extract-and-run --no-appstream \
    --runtime-file "$root/runtime" "$app" "$artifact"
chmod +x "$artifact"
(cd build && sha256sum "$(basename "$artifact")" > "$(basename "$artifact").sha256")
sha256sum "$app/usr/bin/glance" "$libs"/*.so.* > "$root/payload-sha256.txt"
wc -c "$artifact"
echo "Artifact: $artifact; evidence: $root/{ldd,smoke,payload-sha256}.txt"
