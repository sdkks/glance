# Installing Glance

Installation and source-build instructions for macOS and Linux. For everyday controls, see [Using Glance](usage.md). For distribution builds, see [Packaging](packaging.md).

## macOS releases

Download the Apple Silicon DMG or zip from the [releases page](https://github.com/sdkks/glance/releases). macOS requires 11.0 or newer; there is no Intel or Universal build. Oldest-OS runtime compatibility has not been verified.

To install, extract the zip or mount the DMG, then drag `Glance.app` to
`/Applications` (or copy it there). Gatekeeper will still warn on first launch of
a downloaded app: right-click `Glance.app` → Open, then click Open again. As an
alternative, after verifying the source and checksum, remove only this app's
quarantine attribute, then launch it normally:

```sh
xattr -d com.apple.quarantine /Applications/Glance.app
```

These first-launch steps are needed only because the build lacks a Developer ID
signature and notarization; an ad-hoc signature checks bundle integrity but does
not establish a verified developer identity.

## Linux AppImage

Download the x86_64 AppImage and its checksum from the
[releases page](https://github.com/sdkks/glance/releases). Ubuntu 24.04 is the
baseline, not arbitrary older Linux. Check the downloaded checksum, make the
AppImage executable and pass an image path when launching it. Use the version
and architecture you downloaded in place of the example values:

```sh
sha256sum -c Glance-0.1.0-aarch64.AppImage.sha256 # adapt version/architecture
chmod +x Glance-0.1.0-aarch64.AppImage
./Glance-0.1.0-aarch64.AppImage /absolute/path/to/image.avif
```

Normal mounting needs working FUSE (`/dev/fuse`). Without FUSE, use
`--appimage-extract-and-run` or extract and run `squashfs-root/AppRun`.
The pinned runtime embeds libfuse. AppImage desktop integration is not automatic;
use your desktop's integration tool or **Open With**. The embedded desktop entry
is not installed into the host application database. For default associations,
see [default-viewer setup](usage.md#making-glance-the-default-image-viewer).
For runtime dependencies, see
[Linux AppImage packaging](packaging.md#linux-appimage-packaging). For verification,
see the [Linux verification recipe](development.md#linux-verification-recipe).

## Building from source

The pinned Rust toolchain is **1.97.1**, with rustfmt and Clippy. Install it yourself
before running checks; gates do not install tools, update the lockfile, or fix files.
`Cargo.lock` is committed. Linux builds need the normal windowing/graphics system
libraries and a working X11 or Wayland desktop; this increment has only been
smoke-tested on macOS arm64, not certified on Linux.

macOS source builds need Apple command-line tools. For Linux development-library
installation commands, see the prerequisites in
[Linux AppImage packaging](packaging.md#linux-appimage-packaging).
Native AVIF dependencies are listed below; if they are unavailable, compilation
succeeds but AVIF opening reports the named unavailable-codec error.

From the repository root:

```sh
make build
./target/release/glance /absolute/path/to/image.png
cargo run --locked -- /absolute/path/to/folder
```

For contributor checks and Make targets, see [Development](development.md).

## Native AVIF build dependencies

The default `avif-native` feature probes pkg-config on macOS/Linux. This adapter
requires libavif ≥1.4.2 and dav1d ≥1.5.4. When absent, compilation succeeds with
`AVIF support not built: libavif/dav1d not found` and opening AVIF reports the
same named message. `cargo build --no-default-features` explicitly disables it.
AVIF remains an eligible extension because the dispatch provides this specific
unavailable-codec error. Header and library paths must describe the same ABI.

```sh
# macOS development dependencies
brew install pkg-config libavif dav1d
# Linux: repositories must provide the minimum versions above
sudo apt install pkg-config libavif-dev libdav1d-dev
```

Validated development versions: Homebrew libavif 1.4.2 + dav1d 1.5.4, macOS
arm64. The Homebrew libavif dylib also links aom/libvmaf: **this is not the shipping
artifact**. The [release payload](packaging.md#native-avif-release-payload-macos) replaces it with unmodified upstream
libavif built with dav1d only; it excludes external libyuv, encoders, libvmaf and
libheif, but retains upstream's embedded BSD-3-Clause scaling subset.
Linux release bundling remains unimplemented.

## Installing on Linux (source)

With the Rust toolchain and Linux development libraries described in [Building from source](#building-from-source) installed:

```sh
make install-linux DRY_RUN=1  # prints build/filesystem/cache actions; writes nothing
make install-linux           # builds release; defaults to PREFIX="$HOME/.local"
export PATH="$HOME/.local/bin:$PATH"
glance /absolute/path/to/image.png
make uninstall-linux DRY_RUN=1
make uninstall-linux
```

`PREFIX` must be an absolute canonical path (no dot components or repeated
slashes). Existing ancestors of the combined destination and installed directories
must not be symlinks; use physical paths (for example, `pwd -P`) when necessary.
Override `PREFIX` with `make install-linux PREFIX=/absolute/path`.
`DESTDIR` is an optional absolute staging root prepended to `PREFIX`, for example
`make install-linux PREFIX=/usr DESTDIR=/tmp/glance-stage`. Use the **same**
`PREFIX` and `DESTDIR` for uninstall. Unstaged system prefixes such as `/usr` and
`/usr/local` require explicit `SUDO=1` confirmation; this flag never runs sudo
or grants permissions. The installer refuses to overwrite files without its
own existing manifest. Reinstallation validates every manifest-owned file before
building or writing anything, refusing if any file is missing, modified, or
replaced; unchanged installations can be reinstalled.

The installed layout is `bin/glance`, `share/applications/glance.desktop`
(`Exec=glance %f`, `TryExec=glance`, `Icon=glance`), and
`share/icons/hicolor/{16,24,32,48,64,128,256}x{same}/apps/glance.png`.
Supported image MIME associations are advertised; AVIF requires the native
libraries described above. No custom MIME XML or default-app override is installed.
Your desktop must search this prefix, and `glance` must be on its session PATH;
a custom prefix is not automatically a desktop search path.

`share/glance/install-manifest` records checksums and exact installed paths.
Uninstall validates the entire manifest and refuses to delete anything if a file
was modified or replaced. A missing manifest warns and leaves everything alone.
Only these files and the manifest are deleted, **not shared directories**, user
settings, or other applications. Empty directories intentionally remain.
`update-desktop-database` and `update-mime-database` are optional, nonfatal, and
run only for an unstaged `$HOME/.local` install/uninstall; MIME refresh also
requires an existing `share/mime` directory. Custom/staged prefixes skip caches.


See [Linux source-install verification](development.md#linux-source-install-verification) for association checks and the install/uninstall smoke recipe. See [default-viewer setup](usage.md#making-glance-the-default-image-viewer) for desktop registration details.
