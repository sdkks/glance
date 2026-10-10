#!/bin/sh
# Ad-hoc signed, host-architecture macOS artifacts. No tools/dependencies are installed.
set -eu
cd "$(dirname "$0")/.."
[ "$(uname -s)" = Darwin ] || { echo 'macOS packaging requires macOS.' >&2; exit 1; }
arch=$(uname -m)
case "$arch" in
    arm64|x86_64) ;;
    *) echo "Unsupported macOS architecture: $arch" >&2; exit 1 ;;
esac
version=$(awk '/^\[package\]$/ { package=1; next } /^\[/ { package=0 } package && /^version = / { gsub(/"/, "", $3); print $3; exit }' Cargo.toml)
case "$version" in
    ''|*[!0-9.]*) echo 'Packaging requires a numeric Cargo package version (x.y.z).' >&2; exit 1 ;;
esac
# 11.0 is the first Apple Silicon macOS and a common floor for both architectures.
export MACOSX_DEPLOYMENT_TARGET=11.0
native_stage=${1:-${AVIF_STAGE:-build/bundle-libs}}
notices=${NOTICES_STAGE:-build/notices}
# A skipped native stage is a genuinely source-only build, not Homebrew linkage.
if [ "${SKIP_AVIF:-0}" = 1 ]; then
    cargo build --release --locked --no-default-features
    binary=target/release/glance
else
    binary="$native_stage/Contents/MacOS/glance"
    for input in "$binary" "$native_stage/Contents/Frameworks/libavif.16.dylib" \
        "$native_stage/Contents/Frameworks/libdav1d.7.dylib" \
        "$native_stage/Contents/Resources/licenses/native/libavif-1.4.2-LICENSE" \
        "$native_stage/Contents/Resources/licenses/native/dav1d-1.5.4-COPYING"; do
        [ -f "$input" ] || { echo "Missing AVIF stage input: $input; run make bundle-avif (or SKIP_AVIF=1)." >&2; exit 1; }
    done
fi
if [ "${SKIP_NOTICES:-0}" != 1 ]; then
    [ -f "$notices/THIRD-PARTY-NOTICES.md" ] && [ -f "$notices/LICENSE" ] || {
        echo "Missing notices: $notices; run make notices (or SKIP_NOTICES=1)." >&2; exit 1;
    }
fi
out=${MACOS_OUT:-"$PWD/target/macos-artifacts"}
mkdir -p "$out"
stage=$(mktemp -d /tmp/glance-macos-package.XXXXXX)
trap 'rm -rf "$stage"' EXIT HUP INT TERM
app="$stage/Glance.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/glance"
cp resources/glance.icns "$app/Contents/Resources/glance.icns"
if [ "${SKIP_NOTICES:-0}" != 1 ]; then
    ditto "$notices" "$app/Contents/Resources/licenses"
fi
if [ "${SKIP_AVIF:-0}" != 1 ]; then
    mkdir -p "$app/Contents/Frameworks" "$app/Contents/Resources/licenses/native"
    cp "$native_stage/Contents/Frameworks/libavif.16.dylib" \
        "$native_stage/Contents/Frameworks/libdav1d.7.dylib" "$app/Contents/Frameworks/"
    # The generated notices already contain this tree. Refuse disagreements,
    # then merge only identical/new files (versions.txt belongs to the generator).
    find "$native_stage/Contents/Resources/licenses/native" -type f > "$stage/native-files"
    while IFS= read -r file; do
        relative=${file#"$native_stage/Contents/Resources/licenses/native/"}
        destination="$app/Contents/Resources/licenses/native/$relative"
        if [ -f "$destination" ]; then
            cmp "$file" "$destination" || { echo "Conflicting native notice: $relative" >&2; exit 1; }
        else
            mkdir -p "$(dirname "$destination")"
            cp "$file" "$destination"
        fi
    done < "$stage/native-files"
fi
for macho in "$app/Contents/MacOS/glance" "$app"/Contents/Frameworks/*.dylib; do
    [ -f "$macho" ] || continue
    lipo "$macho" -verify_arch "$arch"
    otool -L "$macho" | awk 'NR > 1 { print $1 }' > "$stage/dependencies"
    while IFS= read -r dep; do
        case "$dep" in
            /System/Library/*|/usr/lib/*) ;;
            @rpath/*|@executable_path/../Frameworks/*|@loader_path/*)
                [ -f "$app/Contents/Frameworks/${dep##*/}" ] || {
                    echo "Missing bundled dependency: $dep ($macho)" >&2; exit 1;
                } ;;
            *)
                echo "Nonrelocatable native dependency: $dep ($macho)." >&2
                exit 1 ;;
        esac
    done < "$stage/dependencies"
done
plist="$app/Contents/Info.plist"
cat > "$plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Glance</string>
<key>CFBundleDisplayName</key><string>Glance</string>
<key>CFBundleIdentifier</key><string>io.github.glance-viewer.glance</string>
<key>CFBundleExecutable</key><string>glance</string>
<key>CFBundleIconFile</key><string>glance.icns</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundleVersion</key><string>$version</string>
<key>LSMinimumSystemVersion</key><string>11.0</string>
<key>LSApplicationCategoryType</key><string>public.app-category.graphics-design</string>
<key>CFBundleDocumentTypes</key><array>
EOF
# Keep these extension groups aligned with src/formats.rs. Imported declarations
# describe other applications' formats; none is exported/claimed as our own.
types='public.jpeg:jpg,jpeg public.png:png public.tiff:tif,tiff com.microsoft.bmp:bmp com.microsoft.ico:ico com.microsoft.cur:cur com.apple.icns:icns com.truevision.tga-image:tga public.pnm:pnm,pbm,pgm,ppm,pam public.wbmp:wbmp public.xbm:xbm public.xpm:xpm public.pcx:pcx public.sgi-image:sgi,bw,rgb,rgba com.compuserve.gif:gif org.webmproject.webp:webp public.avif:avif public.jpeg-xl:jxl public.svg-image:svg public.svgz:svgz com.adobe.photoshop-image:psd org.krita.kra:kra org.openraster.ora:ora'
for type in $types; do
    uti=${type%%:*}
    printf '<dict><key>CFBundleTypeName</key><string>%s</string><key>CFBundleTypeRole</key><string>Viewer</string><key>LSHandlerRank</key><string>Alternate</string><key>LSItemContentTypes</key><array><string>%s</string></array></dict>\n' "$uti" "$uti" >> "$plist"
done
printf '</array><key>UTImportedTypeDeclarations</key><array>\n' >> "$plist"
for type in $types; do
    uti=${type%%:*}
    extensions=${type#*:}
    printf '<dict><key>UTTypeIdentifier</key><string>%s</string><key>UTTypeConformsTo</key><array><string>public.image</string></array><key>UTTypeTagSpecification</key><dict><key>public.filename-extension</key><array>' "$uti" >> "$plist"
    old_ifs=$IFS; IFS=,
    for ext in $extensions; do printf '<string>%s</string>' "$ext" >> "$plist"; done
    IFS=$old_ifs
    printf '</array></dict></dict>\n' >> "$plist"
done
printf '</array></dict></plist>\n' >> "$plist"
plutil -lint "$plist"
# Sign inside out, after all bundle contents are final and before archiving.
sign_bundle() {
    command -v codesign >/dev/null 2>&1 || {
        echo 'codesign is required to sign macOS artifacts.' >&2; return 1;
    }
    for lib in "$app"/Contents/Frameworks/*.dylib; do
        [ -f "$lib" ] || continue
        codesign --force --sign - "$lib" || return 1
    done
    codesign --force --sign - "$app/Contents/MacOS/glance" &&
        codesign --force --sign - "$app" &&
        codesign --verify --deep --strict --verbose=2 "$app" &&
        codesign -dv "$app"
}
if ! sign_bundle; then
    if [ "${ALLOW_UNSIGNED:-0}" = 1 ]; then
        echo 'WARNING: signing failed; ALLOW_UNSIGNED=1 permits artifacts without a verified signature.' >&2
    else
        echo 'macOS signing failed; refusing to ship artifacts without a verified signature (override: ALLOW_UNSIGNED=1).' >&2
        exit 1
    fi
fi
ln -s /Applications "$stage/Applications"
name="Glance-$version-$arch"
ditto -c -k --sequesterRsrc --keepParent "$app" "$stage/$name.zip"
# Use a separate image-root so logs and archives cannot leak into the DMG.
mkdir "$stage/image-root"
ditto "$app" "$stage/image-root/Glance.app"
ln -s /Applications "$stage/image-root/Applications"
hdiutil create -volname "Glance $version" -srcfolder "$stage/image-root" \
    -format UDZO -imagekey zlib-level=9 "$stage/$name.dmg"
(cd "$stage" && shasum -a 256 "$name.zip" "$name.dmg" > "$name.sha256")
# Preserve a previous direct app instead of merging stale libraries/notices.
if [ -e "$out/Glance.app" ]; then
    previous=$(mktemp -d "$out/previous.XXXXXX")
    mv "$out/Glance.app" "$previous/Glance.app"
    echo "Previous app preserved: $previous/Glance.app"
fi
ditto "$app" "$out/Glance.app"
for ext in zip dmg sha256; do mv -f "$stage/$name.$ext" "$out/$name.$ext"; done
printf 'Artifacts: %s/Glance.app and %s/%s.{zip,dmg,sha256}\n' "$out" "$out" "$name"
app_bytes=$(find "$out/Glance.app" -type f -exec stat -f '%z' {} + | awk '{ total += $1 } END { printf "%.0f", total }')
printf 'Glance.app payload: %s bytes\n' "$app_bytes"
stat -f '%N: %z bytes' "$out/$name.zip" "$out/$name.dmg"
dmg_bytes=$(stat -f '%z' "$out/$name.dmg")
if [ "$dmg_bytes" -lt 40000000 ]; then budget=PASS; else budget=OVER; fi
printf 'DMG budget: %s (< 40 MB; UDZO zlib-level=9)\n' "$budget"
