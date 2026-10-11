# Changelog

All notable changes to Glance will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Add **Help → About Glance** with version, copyable commit SHA and build details.

- Confirmed **Settings → Set as the default image viewer…** action for supported
  image types on macOS 12+ and Linux, with success/failure notices.

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

- Accept local `file://` references passed by launchers as the initial image path.

- Register the macOS document-open handler from AppKit launch notifications,
  with an idempotent viewer fallback; `GLANCE_TRACE_OPEN=1` identifies each
  registration point (cold-launch document delivery remains unverified).

- Automatically rebuild stale macOS native AVIF stages safely under `build/`
  during `make macos-artifacts`; direct packaging still refuses stale executables
  and shows commands to rebuild the stage into a fresh path.

- Handle macOS Finder double-click and drag-to-Dock document opening, both at
  launch and while Glance is running (native event delivery remains unverified).
