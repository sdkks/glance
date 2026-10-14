# Changelog

All notable changes to Glance will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Prepare the next GIF/APNG/WebP frame's rendering levels while the current frame
  uploads, using the existing single lookahead slot and unchanged memory limits.

- Upload multiple visible image tiles per byte-bounded batch rather than one per
  UI pass, preserving pixel quality, the staging budget and complete-frame admission.

- Decode animated WebP with bundled libwebp, retaining the existing decoder for
  canvases above 8,294,400 pixels or total encoded durations above 2,147,483,647 ms.
  Static WebP and other formats keep their existing decoders.

### Fixed

- Keep animations moving when frame preparation cannot match their requested rate,
  instead of repeatedly discarding late frames and leaving a still image.

## [0.4.1] - 2026-10-13

### Fixed

- Show the Glance icon on image formats macOS does not declare (Krita,
  OpenRaster, PCX, SGI, PNM, WBMP, XBM, XPM, CUR and compressed SVG), by
  exporting those type declarations with the bundle icon; macOS previously
  rendered a generic page icon for these files. System-declared formats keep
  their standard icons and thumbnails.

## [0.4.0] - 2026-10-11

### Added

- Support trackpad pinch zoom at the pointer, using the same zoom limits as `+`/`−`.

## [0.3.0] - 2026-10-11

### Added

- Add `b` to cycle white/grey/black behind transparent pixels for the current
  image, with a persisted default in `Settings ▸ Background color`. Black keeps
  the existing near-black appearance; opaque images are unchanged.

## [0.2.1] - 2026-10-11

### Fixed

- Avoid spurious `File changed` errors when only file metadata changes during
  image decoding or animation playback, or the initial file-version read fails.
  Continue rejecting observed content changes during decoding and playback.

## [0.2.0] - 2026-10-11

### Added

- Add `Settings ▸ Open new files in` with persisted `Current window` (default) and
  opt-in `New window` choices for macOS documents delivered while Glance is running
  (Finder double-click or `open -a`). Current window prefers a window already
  showing the file, then the focused window; New window leaves other windows
  untouched. A multi-file request opens only its first document; startup still
  opens the launch document in the first window.

- Add `File ▸ New Window` for independent image windows, each with its own folder
  position, zoom, pan and playback; settings and clipboard remain app-wide.
  `⌘W` closes the focused window; `⌘Q` quits all windows, and closing the last
  window quits Glance. Image and cache resource bounds apply per window, so
  memory use scales with the number of open windows.

### Fixed

- Offer `Settings ▸ Open new files in` only on macOS, where incoming documents
  can be delivered to an already-running instance.

- Keep install examples version-agnostic and generate DMG instructions with the
  packaged version instead of embedding a stale release version.

## [0.1.4] - 2026-10-11

### Fixed

- Correct macOS installation and first-launch guidance: recommend `gh`/`curl`
  downloads with checksum checks, retain Terminal and System Settings routes
  for browser downloads, explain notarization, and clarify that a quarantined
  DMG helper must be run from Terminal rather than double-clicked.

## [0.1.3] - 2026-10-11

### Fixed

- Release packaging installs the pinned toolchain's rustfmt and clippy components up
  front, so tagged macOS builds no longer fail on a cargo-fmt conflict.

## [0.1.2] - 2026-10-11

### Added

- Add a DMG install helper and first-launch instructions for self-signed,
  non-notarized macOS builds.

## [0.1.1] - 2026-10-11

### Changed

- Documentation is reorganised for readers and contributors: the README is now a
  short quickstart, the detail lives under `docs/`, and a new `CONTRIBUTING.md`
  explains local checks, pull requests and how releases are cut.

## [0.1.0] - 2026-10-11

### Added

- Help → About Glance, with the version, a copyable commit SHA and build details.
- Set Glance as the default image viewer for supported image types from Settings
  (macOS 12+ and Linux), with clear success and failure notices.
- Image-first desktop viewer with file/folder opening, drag and drop, refreshed
  folder navigation, natural/size/date sorting, zoom, Fit, Actual Size and Space-pan.
- Raster and icon previews, scale-aware SVG/SVGZ, AVIF and JPEG XL stills, and
  flattened PSD/Krita/OpenRaster document previews. Native AVIF requires libavif
  and dav1d; EXR/Radiance HDR, PQ/HLG and AVIF/JXL sequences remain unsupported.
- Tiled full-detail rendering, dyadic minification, atomic image swaps, ICC-to-sRGB
  normalization, retained U16 sources, and disclosed float-TIFF SDR previews.
- GIF/APNG/WebP playback with pause, frame stepping, browser-compatible timing
  and configurable animation rate.
- Non-destructive rotate/mirror/flip, transformed image and file clipboard actions,
  local file-reference paste, and reveal in the file manager.
- Bounded neighbor prefetch and resident history, persisted preload depth and
  animation rate, and platform app icons derived from the human-supplied master.

### Fixed

- Open images from Finder by double-click or drag to the Dock, both when Glance is
  launched by the document and while it is already running.
- Accept `file://` references passed by launchers as the initial image path.
- `make macos-artifacts` rebuilds a stale native AVIF stage automatically instead of
  refusing to package.
