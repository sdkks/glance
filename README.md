# glance

An independently authored MIT desktop image viewer, built with Rust and
eframe/egui's wgpu renderer. Opens raster, icon and flattened document images;
it is not yet the complete planned viewer.

qView was studied as prior art for its interaction model and feature set. Glance
shares no qView code or assets and is not derived from it; it is an independent
Rust implementation under its own MIT license.

## Quickstart

### Install on macOS

Download the Apple Silicon DMG or zip from the
[releases page](https://github.com/sdkks/glance/releases).
There is no Intel or Universal build. The declared minimum is macOS 11.0;
see [installation notes](docs/installing.md#macos-releases) for compatibility caveats.

Mount the DMG or extract the zip, then drag `Glance.app` to Applications.
The app is ad-hoc signed, not Developer ID signed or notarized.
On first launch, right-click `Glance.app` → **Open**, then click **Open** again.
See [Installing Glance](docs/installing.md) for checksum and Gatekeeper details.

### Install on Linux

Download the x86_64 AppImage and its checksum from the
[releases page](https://github.com/sdkks/glance/releases).
Ubuntu 24.04 is the baseline. Make the AppImage executable and run it with
an image path. For example, from the download directory:

```sh
chmod +x Glance-0.1.0-x86_64.AppImage
./Glance-0.1.0-x86_64.AppImage /absolute/path/to/image.png
```

Use the version you downloaded in place of `0.1.0`.
AppImage desktop integration is not automatic. See
[Linux installation](docs/installing.md#linux-appimage) for checksums,
FUSE alternatives and desktop registration. You can also
[install from source](docs/installing.md#installing-on-linux-source).

### Build from source

From a checkout, install the pinned Rust **1.97.1** toolchain with rustfmt and
Clippy. macOS needs Apple command-line tools; Linux needs windowing and graphics
development libraries and a working X11 or Wayland desktop.
Native AVIF uses pkg-config, libavif ≥1.4.2 and dav1d ≥1.5.4.
Without those native libraries, the build succeeds but AVIF is unavailable.
See [build prerequisites](docs/installing.md#building-from-source).

```sh
make build
./target/release/glance /absolute/path/to/image.png
cargo run --locked -- /absolute/path/to/folder
```

### Open a file or folder

Pass an image or folder path when launching Glance. For a development run:

```sh
make run ARGS='/absolute/path/to/image.png'
# Or:
cargo run --locked -- /absolute/path/to/folder
```

In the app, choose **File → Open…**, press **⌘O / Ctrl+O**, or drop a file
or folder onto the window. A folder opens its first eligible image.
Browsing is nonrecursive and skips dot-prefixed and Finder-hidden names.

## Using Glance

### Open, navigate and inspect

Use **Left/Right** for the previous/next image and **Home/End** for first/last.
Navigation does not wrap. The title shows the filename and position/count.
Choose **Navigate → Sort** to sort by natural name, size or modification time.
Sort lasts for the session; navigation refreshes the folder's file list.

Images start centered at Fit without enlarging small images.
Choose **View → Actual Size (100%)** to inspect one source pixel per physical
screen pixel. Zoom with `+`/`−` or the mouse wheel. Drag to pan an enlarged
image, or hold **Space** and move the mouse without pressing a button.
Trackpad scrolling pans; scrolling never navigates.

While another file loads, the previous image stays visible. A failed load
keeps it visible too, with a named error and navigation still available.
See [view controls](docs/usage.md#opening-navigation-and-view-controls)
for focus rules, sorting details and reload behavior.

### Keyboard shortcuts

Use ⌘ on macOS and Ctrl on Linux for the platform shortcuts below.
Viewer shortcuts are suppressed while menus or another control own focus.

| Action | Shortcut |
| --- | --- |
| Open | ⌘O / Ctrl+O |
| Close the viewer | ⌘W / Ctrl+W |
| Previous / next | Left / Right |
| First / last | Home / End |
| Fit | `0` or `1` |
| Actual Size (100%) | `2` |
| Zoom in / out | `+` (or `=`) / `−` |
| Temporary pan | Hold Space and move the mouse |
| Rotate clockwise / counterclockwise | `R` / `Shift+R` |
| Mirror horizontally / flip vertically | `M` / `F` |
| Pause / resume animation | `P` |
| Next animation frame | `.` |

The stack also provides **⌘Q / Ctrl+Q** as a global Quit shortcut.
Menus support Tab/Shift+Tab to focus and Enter/Space to activate.

### Animation and settings

GIF, 8-bit APNG and animated WebP start playing when opened.
Use **Playback → Pause/Resume** or **Next Frame** to inspect them.
**Settings → Animation rate** offers file timing, fixed frame rates,
and **Off (first frame only)**. Space remains temporary pan, not playback.

**Settings → Preload depth** controls neighbor decoding and resident history.
Preload depth and animation rate are saved on macOS and Linux; sort is not.
See [animation playback](docs/usage.md#animation-playback),
[preloading](docs/usage.md#preloading-and-resident-history) and
[settings persistence](docs/usage.md#settings-persistence) for details.

### Clipboard, reveal and transforms

The File and right-click menus offer **Copy Image**, **Copy File**, **Paste File**
and **Reveal in File Manager**. Copy Image copies the complete displayed image
with its orientation and transforms, not a viewport crop. High-depth images
become an 8-bit sRGB clipboard bitmap only when copied.
Paste File accepts local file references, not raw bitmaps or remote URLs.
There is no advertised ⌘V / Ctrl+V file-paste shortcut.

Use the View menu to rotate, mirror or flip without changing the file.
**Reset View** removes user transforms, restores embedded orientation and fits
again. Transforms reset when a different file becomes ready to display.
See [clipboard and desktop actions](docs/usage.md#clipboard-and-desktop-actions)
for platform behavior and validation caveats.

To make Glance the default viewer, choose
**Settings → Set as the default image viewer…** and confirm.
See [default-viewer setup](docs/usage.md#making-glance-the-default-image-viewer)
for platform requirements, desktop registration and manual alternatives.

### Supported formats

Glance opens JPEG, PNG, TIFF, BMP, GIF, WebP, common icon and legacy raster
formats, JPEG XL stills and static SVG/SVGZ. AVIF stills require native decoder
support, bundled in release packages. PSD, Krita and OpenRaster open their
stored flattened composites, not editable layers.

Support depends on the variant, not just the extension. EXR/Radiance HDR,
PQ/HLG, XCF and AVIF/JXL sequences remain unsupported.
See [formats and color handling](docs/formats.md) for the full tables,
16-bit handling, ICC behavior and format-specific limits.

## Limits and troubleshooting

Large files can hit encoded-input, decoded-image or rendering limits.
Platform desktop behavior and older-OS compatibility still have validation gaps.
See [Rendering and limits](docs/rendering.md) and the platform verification
recipes in [Development](docs/development.md#verifying-artifacts-and-installs).

## Documentation

- [Contributing](CONTRIBUTING.md) — local setup, checks, pull requests and bug reports.

- [Installing Glance](docs/installing.md) — downloads, first launch and source builds.
- [Using Glance](docs/usage.md) — controls, playback, settings and desktop actions.
- [Supported formats and color](docs/formats.md) — formats, depth and codec caveats.
- [Rendering and limits](docs/rendering.md) — full detail, minification and resource bounds.
- [Development](docs/development.md) — quality gates, audits, hooks and icons.
- [Packaging](docs/packaging.md) — artifact construction and distribution notices.
- [Releasing Glance](docs/releasing.md) — maintainer-only versioning, CI and release steps.

## License

Glance is MIT; see [LICENSE](LICENSE).
See [THIRD_PARTY.md](THIRD_PARTY.md) for dependency licenses, artwork provenance
and distribution limitations, and [Licensing](docs/packaging.md#licensing)
for the dependency summary.
