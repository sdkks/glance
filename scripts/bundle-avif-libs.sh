#!/bin/bash
# Stage a relocatable macOS native AVIF release payload, not a complete .app.
set -euo pipefail
cd "$(dirname "$0")/.."
[[ $(uname -s) == Darwin ]] || { echo 'Linux bundling is not implemented; see README.' >&2; exit 1; }
for tool in brew pkg-config cmake curl shasum tar otool install_name_tool codesign zip cargo; do
    command -v "$tool" >/dev/null || { echo "Missing prerequisite: $tool" >&2; exit 1; }
done
out=${1:-build/bundle-libs}
mkdir -p "$(dirname "$out")"
# Never overwrite another packaging run or caller-owned files.
mkdir "$out" || { echo 'Choose a new/empty staging path.' >&2; exit 1; }
out=$(cd "$out" && pwd)
work="$out/native-build"
libs="$out/Contents/Frameworks"
notices="$out/Contents/Resources/licenses/native"
mkdir -p "$work" "$libs" "$notices" "$out/Contents/MacOS"
avif=$(brew --prefix libavif)
dav1d=$(brew --prefix dav1d)
export PKG_CONFIG_PATH="$avif/lib/pkgconfig:$dav1d/lib/pkgconfig"
[[ $(pkg-config --modversion libavif) == 1.4.2 ]]
[[ $(pkg-config --modversion dav1d) == 1.5.4 ]]
otool -L "$avif/lib/libavif.dylib" "$dav1d/lib/libdav1d.dylib" > "$out/before.txt"
# The stock libavif bottle links AOM (and transitively vmaf). Rebuild the
# unmodified, checksum-pinned upstream source with only the dav1d backend.
# AVIF_LIBYUV=OFF disables external libyuv, not upstream's embedded BSD-3
# scaling subset: upstream CMake still compiles five third_party/libyuv files.
curl --fail --location --retry 3 https://github.com/AOMediaCodec/libavif/archive/refs/tags/v1.4.2.tar.gz -o "$work/libavif.tar.gz"
printf '%s  %s\n' 2b645287340ba5a631d268b551dc2d72bd73ac33335962dd36dcdb6d8366921d "$work/libavif.tar.gz" | shasum -a 256 -c -
tar -xzf "$work/libavif.tar.gz" -C "$work"
cmake -S "$work/libavif-1.4.2" -B "$work/cmake" \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$work/prefix" \
    -DBUILD_SHARED_LIBS=ON -DAVIF_CODEC_DAV1D=SYSTEM \
    -DAVIF_CODEC_AOM=OFF -DAVIF_CODEC_AVM=OFF -DAVIF_CODEC_LIBGAV1=OFF \
    -DAVIF_CODEC_RAV1E=OFF -DAVIF_CODEC_SVT=OFF -DAVIF_LIBYUV=OFF \
    -DAVIF_LIBSHARPYUV=OFF -DAVIF_BUILD_APPS=OFF \
    -DAVIF_BUILD_EXAMPLES=OFF -DAVIF_BUILD_TESTS=OFF
cmake --build "$work/cmake" --parallel
cmake --install "$work/cmake"
cp -L "$work/prefix/lib/libavif.16.dylib" "$libs/"
cp -L "$dav1d/lib/libdav1d.7.dylib" "$libs/"
# Both approved libraries' complete non-system closure must fit this allowlist.
# Unexpected dependencies fail closed rather than silently shipping encoders.
deps() { otool -L "$1" | awk 'NR > 1 {print $1}'; }
for file in "$libs"/*.dylib; do
    while IFS= read -r dep; do
        case "$dep" in
            /usr/lib/*|/System/Library/*) ;;
            */libavif.16.dylib|*/libdav1d.7.dylib)
                install_name_tool -change "$dep" "@loader_path/$(basename "$dep")" "$file" ;;
            *) echo "Unapproved native dependency: $dep" >&2; exit 1 ;;
        esac
    done < <(deps "$file")
    install_name_tool -id "@loader_path/$(basename "$file")" "$file"
    codesign --force --sign - "$file"
done
cp "$work/libavif-1.4.2/LICENSE" "$notices/libavif-1.4.2-LICENSE"
cp "$avif/LICENSE" "$notices/libavif-1.4.2-Homebrew-LICENSE"
cp "$dav1d/COPYING" "$notices/dav1d-1.5.4-COPYING"
cp "$work/libavif-1.4.2/third_party/libyuv/AUTHORS" "$notices/embedded-libyuv-AUTHORS"
(cd "$work/libavif-1.4.2" && shasum -a 256 third_party/libyuv/source/*.c) > "$notices/embedded-libyuv-source-sha256.txt"
shasum -a 256 "$dav1d/lib/libdav1d.7.dylib" > "$out/dav1d-input-sha256.txt"
for formula in libavif dav1d; do
    prefix=$(brew --prefix "$formula")
    cp "$prefix/INSTALL_RECEIPT.json" "$notices/$formula-Homebrew-receipt.json"
    cp "$prefix/sbom.spdx.json" "$notices/$formula-Homebrew-sbom.spdx.json"
done
export PKG_CONFIG_PATH="$work/prefix/lib/pkgconfig:$dav1d/lib/pkgconfig"
cargo build --release --locked
binary="$out/Contents/MacOS/glance"
cp target/release/glance "$binary"
stat -f 'original executable bytes: %z' "$binary" > "$out/sizes.txt"
found=0
while IFS= read -r dep; do
    case "$dep" in
        */libavif.16.dylib)
            install_name_tool -change "$dep" '@executable_path/../Frameworks/libavif.16.dylib' "$binary"
            found=1 ;;
        /usr/lib/*|/System/Library/*) ;;
        *) echo "Unapproved app dependency: $dep" >&2; exit 1 ;;
    esac
done < <(deps "$binary")
[[ $found == 1 ]] || { echo 'Release binary has no native AVIF linkage.' >&2; exit 1; }
codesign --force --sign - "$binary"
otool -L "$binary" "$libs"/*.dylib > "$out/after.txt"
if grep -E '/opt/homebrew|/usr/local|/Cellar/' "$out/after.txt"; then
    echo 'Nonrelocatable library reference remains.' >&2; exit 1
fi
# Empty override/fallback paths alone cannot disprove an absolute install name.
# Audit the entire closure above AND trace actual dyld loads during real decode.
mkdir "$out/empty-loader-path"
(
    unset DYLD_INSERT_LIBRARIES DYLD_FRAMEWORK_PATH DYLD_FALLBACK_FRAMEWORK_PATH
    unset DYLD_VERSIONED_LIBRARY_PATH DYLD_VERSIONED_FRAMEWORK_PATH DYLD_ROOT_PATH
    DYLD_LIBRARY_PATH="$out/empty-loader-path" DYLD_FALLBACK_LIBRARY_PATH="$out/empty-loader-path" \
        DYLD_PRINT_LIBRARIES=1 GLANCE_SMOKE_IMAGE=1 \
        "$binary" "$PWD/assets/test/web/alpha8.avif" > "$out/smoke.txt" 2> "$out/dyld.txt"
)
grep -F 'decoded 5x3' "$out/smoke.txt"
for name in libavif.16.dylib libdav1d.7.dylib; do
    grep -F "$libs/$name" "$out/dyld.txt"
done
if grep -E '/opt/homebrew|/usr/local|/Cellar/' "$out/dyld.txt"; then
    echo 'Smoke loaded package-manager code.' >&2; exit 1
fi
stat -f '%N: %z bytes' "$binary" "$libs"/*.dylib >> "$out/sizes.txt"
(cd "$out/Contents" && zip -q -9 -r "$out/native-avif.zip" Frameworks Resources/licenses/native)
stat -f 'native libraries plus notices zip: %z bytes' "$out/native-avif.zip" >> "$out/sizes.txt"
printf 'libavif 1.4.2; dav1d 1.5.4; arch %s; source sha256 %s\n' "$(uname -m)" \
    2b645287340ba5a631d268b551dc2d72bd73ac33335962dd36dcdb6d8366921d > "$out/versions.txt"
shasum -a 256 "$binary" "$libs"/*.dylib > "$out/payload-sha256.txt"
echo "Staged payload: $out/Contents (copy into .app before final signing/archiving)"
echo "Evidence: $out/{before,after,smoke,dyld,sizes,versions}.txt"
