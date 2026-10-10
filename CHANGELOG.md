# Changelog

All notable changes to Glance will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
