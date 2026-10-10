#!/bin/sh
# Ad-hoc signed, host-architecture macOS artifacts. No tools/dependencies are installed.
set -eu
cd "$(dirname "$0")/.."
[ "$(uname -s)" = Darwin ] || { echo 'macOS packaging requires macOS.' >&2; exit 1; }
arch=$(uname -m)
case "$arch" in
    arm64) target=aarch64-apple-darwin ;;
    x86_64) target=x86_64-apple-darwin ;;
    *) echo "Unsupported macOS architecture: $arch" >&2; exit 1 ;;
esac
version=$(awk '/^\[package\]$/ { package=1; next } /^\[/ { package=0 } package && /^version = / { gsub(/"/, "", $3); print $3; exit }' Cargo.toml)
case "$version" in
    ''|*[!0-9.]*) echo 'Packaging requires a numeric Cargo package version (x.y.z).' >&2; exit 1 ;;
esac
# 11.0 is the first Apple Silicon macOS and a common floor for both architectures.
export MACOSX_DEPLOYMENT_TARGET=11.0
cargo rustc --release --locked --target "$target" --target-dir target --bin glance -- \
    -C link-arg=-Wl,-rpath,@executable_path/../Frameworks
out="$PWD/target/macos-artifacts"
mkdir -p "$out"
stage=$(mktemp -d /tmp/glance-macos-package.XXXXXX)
trap 'rm -rf "$stage"' EXIT HUP INT TERM
app="$stage/Glance.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "target/$target/release/glance" "$app/Contents/MacOS/glance"
cp resources/glance.icns "$app/Contents/Resources/glance.icns"
# Optional native packaging seam: flat, already-relocatable dylibs. Never alter
# install names here; the native producer owns those inputs. Copied binaries
# receive ad-hoc signatures below, before signing the enclosing bundle.
if [ -d bundle-libs ]; then
    mkdir -p "$app/Contents/Frameworks"
    for lib in bundle-libs/*.dylib; do
        [ -f "$lib" ] || continue
        cp -L "$lib" "$app/Contents/Frameworks/"
    done
fi
for macho in "$app/Contents/MacOS/glance" "$app"/Contents/Frameworks/*.dylib; do
    [ -f "$macho" ] || continue
    otool -L "$macho" | awk 'NR > 1 { print $1 }' > "$stage/dependencies"
    while IFS= read -r dep; do
        case "$dep" in
            /System/Library/*|/usr/lib/*) ;;
            @rpath/*|@executable_path/../Frameworks/*|@loader_path/*)
                [ -f "$app/Contents/Frameworks/${dep##*/}" ] || {
                    echo "Missing bundled dependency: $dep ($macho)" >&2; exit 1;
                } ;;
            *)
                if [ -d bundle-libs ]; then
                    echo "Native inputs must use relocatable install names, not $dep ($macho)." >&2
                    exit 1
                fi
                echo "WARNING: development-only external native dependency: $dep" >&2
                ;;
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
    codesign --force --deep --sign - "$app" &&
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
    -format UDZO "$stage/$name.dmg"
(cd "$stage" && shasum -a 256 "$name.zip" "$name.dmg" > "$name.sha256")
# ditto updates the direct-launch bundle; archives replace only these exact files.
ditto "$app" "$out/Glance.app"
for ext in zip dmg sha256; do mv -f "$stage/$name.$ext" "$out/$name.$ext"; done
printf 'Artifacts: %s/Glance.app and %s/%s.{zip,dmg,sha256}\n' "$out" "$out" "$name"
