# Supported formats and color

Format support, color handling and codec boundaries. See [Installing Glance](installing.md#native-avif-build-dependencies) for native AVIF build dependencies and [Rendering and limits](rendering.md) for resource limits.

## Color and depth

Embedded ordinary RGB ICC matrix and LUT profiles are converted to sRGB by pinned
moxcms 0.8.1, using relative-colorimetric intent and explicit gamut clipping.
Untagged RGB is treated as sRGB. Metadata is read before consuming the decoder.
JPEG, PNG, GIF and WebP provide ICC where present; TIFF provides ICC and
orientation; PNG has EXIF-based orientation via the decoder trait default.
WebP ICC and EXIF orientation pass through the same normalization seam. The other
new decoders do not expose embedded ICC/EXIF through their decoder traits (including
nested PNG in ICO/CUR/ICNS); those previews assume sRGB. The title discloses
`ICC fallback: unsupported/invalid profile; assuming sRGB` when parsing/conversion
fails or a profile exceeds our 4 MiB profile/CLUT caps. Alpha bypasses the CMS.
Failed conversions retain original samples, never a partially converted image.
CMYK/YCCK JPEG and CMYK TIFF use the codec's approximate RGB preview, explicitly
labelled `CMYK approximate SDR preview (not profile-accurate)`; their ICC is never
applied to already-converted RGB. This is not monitor-profile-aware output or proofing.

16-bit PNG/TIFF retain straight U16 encoded-sRGB samples through normalization and
for source ownership. Their rendering levels and tiles use RGBA16F **linear-light
premultiplied RGB and linear alpha**. GPU filtering consumes linear half floats
without an extra transfer function or premultiplication. Half precision does not
preserve all 65,536 source values, but the procedural gradient test retains more
than 256 distinct values in the base and reduced levels and gutter payloads.
Final display precision still depends on the SDR attachment/monitor.
Ordinary 8-bit images retain the encoded premultiplied RGBA8 path described in
[Rendering and limits](rendering.md#rendering-minification-and-full-detail).

Float TIFF is a disclosed **scene-linear sRGB assumption** preview: fixed exposure
1, luminance extended Reinhard `Yout = Y × (1 + Y/16) / (1 + Y)`, white point 4,
RGB ratios preserved before clipping to [0,1]. It never meters exposure per frame.
NaN, negative infinity and negative radiance become zero; positive infinity becomes
radiance 4; finite radiance is capped at 1e20 for finite arithmetic. Nonfinite alpha
becomes zero; finite alpha clamps to [0,1]. Float ICC is not applied and is disclosed.
Normalized SDR remains U16 before half-float presentation.

EXR/Radiance HDR codecs are **not enabled in this pinned feature graph**; their
integration and float-EXR release smoke are deferred, not claimed supported. PQ,
HLG and ACES metadata paths are likewise not integrated. Proposed future conventions
remain PQ absolute luminance divided by 203 cd/m² reference white, and HLG fixed
BT.2100 1000-cd/m² display/system gamma 1.2, without per-frame exposure; these are
not verified current behavior. Human visual ICC/photo/alpha checks and Linux
validation remain outstanding.

## Raster and icon formats

Extensions are case-insensitive; the same dispatch map controls folder eligibility.
Content must match its extension. A supported extension is not a promise to decode
every historical variant. Codec failures show `Could not decode image`,
`Could not read file`, `Unsupported image variant`, or the resource-limit error;
the previous displayed image and navigation remain available.

| Format / extensions | Ordinary preview and boundaries | Source depth / metadata |
| --- | --- | --- |
| JPEG: `jpg`, `jpeg` | Baseline/progressive; approximate CMYK/YCCK preview | U8; ICC |
| PNG: `png` | Static PNG and 8-bit APNG playback | U8/U16 static; ICC and EXIF orientation |
| TIFF: `tif`, `tiff` | Primary image; float preview described above | U8/U16/float; ICC and orientation |
| BMP: `bmp` | Codec-supported palettes, RGB/bitfields and RLE; not all DIB variants | U8 output, including packed 16-bit RGB (not 16 bits/channel); no exposed ICC |
| ICO: `ico` | PNG/DIB entries; upstream selects highest declared bit depth, then largest area; ties choose last directory entry | U8; nested profiles not exposed |
| CUR: `cur` | Validated cursor directory; largest area, first entry on ties; PNG/DIB payload wrapped as a single ICO. Zero size fields mean 256. Hotspots are ignored for static display, not mistaken for ICO bit depth | U8; nested profiles not exposed |
| ICNS: `icns` | Largest icon by upstream size/depth score; legacy data and PNG supported; JPEG2000 subimages give a named unsupported-variant error (no JP2 codec) | U8, including downconverted 16-bit nested PNG; nested profiles not exposed |
| TGA: `tga` | Raw/RLE grayscale, palette and RGB where supported; descriptor origin normalized by codec (top/bottom and left/right); no TGA 2.0 color-management integration | U8 output; packed 16-bit pixels are not 16 bits/channel |
| PNM: `pnm`, `pbm`, `pgm`, `ppm`, `pam` | PBM/PGM/PPM ASCII or binary, ordinary PAM tuples; unsupported tuple/depth combinations are named errors | U8/U16 where provided; no ICC; PFM not dispatched |
| WBMP: `wbmp` | Type 0 monochrome; other types/extended headers named unsupported variants | Expanded U8; no ICC |
| XBM: `xbm` | C bitmap text, LSB-first black bits; hotspots irrelevant to static preview | Expanded U8; no ICC |
| XPM: `xpm` | XPM text and named/hex colors supported by decoder; transparency; unsupported syntax/color variants fail by name | U16 RGBA even for ordinary 8-bit colors; no ICC |
| PCX: `pcx` | Codec-supported paletted and planar RGB variants; no claim of every vintage PCX layout | U8 RGB; no ICC |
| SGI: `sgi`, `bw`, `rgb`, `rgba` | Raw/RLE ordinary 1–4-channel images; unsupported colormap/storage variants are named errors | U8/U16; no ICC |
| GIF: `gif` | Composed animation playback; finite/infinite loops | U8; ICC when exposed |
| WebP: `webp` | Lossy/lossless; composed animation playback | U8; ICC and EXIF orientation |

New formats reuse the existing U8/U16 normalization, rendering levels and clipboard
path; no extra HDR or high-depth fidelity guarantee is added. PNM, SGI and XPM
16-bit samples are retained when the decoder exposes them. The tests exercise
ordinary independently generated examples, not an exhaustive variant corpus.

Extra codecs come from pinned `image-extras` 0.1.1, **only** features
`icns`, `wbmp`, `xbm`, `xpm`, `pcx`, `sgi`, with defaults disabled. Its README
explicitly says fuzzing is not a priority and malformed input may "panic or worse";
it discourages fuzz-crash reports except memory-safety violations. These decoders
are not hardened or sandboxed. Glance contains Rust unwinding across decoder
construction, metadata and pixel decode and returns a named malformed-image error.
The default panic hook may still print a diagnostic. This boundary does not catch
allocator aborts, process aborts or memory-safety failures. Encoded reads and output
sizes remain bounded; upstream scratch-allocation limits remain best-effort.

## Document previews

PSD (`psd`) displays the stored merged composite, never a reconstruction from
layers. Ordinary 8-bit RGB and grayscale, including alpha, support raw, PackBits
RLE and ZIP without prediction. The pinned `psd` 0.3.5 merged-composite API is fed
validated raw planes: our bounded adapter expands RLE/ZIP and skips unused layer
records, rather than using the crate's unsupported ZIP or eager layer parser.
Embedded ICC resource 1039 uses the existing color normalization/fallback path.
CMYK, Lab, 16/32-bit samples, ZIP-prediction and PSB content return specific named
unsupported errors. Other color modes and extra spot channels are unsupported;
this is not full Photoshop fidelity or layer inspection. `.psb` is not eligible.

Krita (`kra`) and OpenRaster (`ora`) display only the root `mergedimage.png`
stored PNG composite. `preview.png` is a thumbnail, not an accepted fallback.
KRA is a flattened preview, not full Krita fidelity; neither format renders layers.
PNG ICC, orientation and U8/U16 normalization follow the ordinary PNG path.
Archives are never extracted to disk. Stored/deflated classic ZIP is supported;
ZIP64 is unsupported. At most 4096 entries, a 64 MiB expanded merged PNG and the
ordinary decoded-image limits apply. Absolute, parent-traversal, backslash and
drive-qualified entry names are rejected, including unused entries. Missing or
duplicate merged images and malformed archives show named format errors.
Tests construct original analytic PNG/ZIP and hand-built PSD fixtures; real
Photoshop/Krita documents and Linux desktop display still need human checks.

## Web and vector formats

JPEG XL (`jxl`) stills use pinned `jxl-oxide` 0.12.6's core API (not its image
adapter), with moxcms enabled and no native JXL runtime. The codec converts to
encoded sRGB with relative-colorimetric intent and applies stream orientation
once; Glance does not rotate it again or reapply the original ICC. Output is
quantized to full-range U16 even for 8-bit sources, then uses the existing U16
levels, tiles and 8-bit clipboard conversion. This is not lossless preservation
of arbitrary float/greater-than-16-bit source samples. HDR transfer, CMYK and
animation/sequence profiles give named unsupported-variant errors. Decoder
tracked allocations are capped at 512 MiB; upstream untracked scratch is not
an RSS guarantee. The original test fixture is a constant 3×2 PPM encoded
losslessly by cjxl 0.10.2; no borrowed image assets are used.

SVG/SVGZ (`svg`, `svgz`) use resvg 0.48.0 / usvg 0.48.1 / tiny-skia 0.12.0
for static shapes, text (system fonts), and bounded embedded JPEG/PNG/GIF/WebP.
Scripts/events, animation, foreignObject, nested SVG and external image files
are rejected by name; there is no network or local-file resource resolver.
Each embedded raster is limited to 4096 pixels per side and 64 MiB RGBA;
expanded gzip XML is limited to 64 MiB. Nested raster ICC is not normalized by
this path. Relative sizes use usvg's default viewport (100×100) and SVG viewBox
rules, not a browser viewport. The parsed tree is retained with the resident;
an off-thread, single-slot worker re-rasterizes into the ordinary U8 source,
levels and tiles. Fit follows the fitted physical size (including DPI); Actual
Size uses one SVG unit per physical pixel. Fit/Actual Size changes request the
exact target scale (0.1% tolerance); zoom requests a new raster when the needed
scale exceeds 1.5× the current/requested raster scale, or the raster is over
2.5× oversized. The previous raster stays visible, scaled, until all replacement
visible tiles are ready; obsolete scale/selection revisions cannot replace it.
Copy Image always re-rasterizes at intrinsic size before applying user transforms,
not at the display resolution. Raster requests are checked before allocation:
16384 pixels per side and 512 MiB straight RGBA8 (134,217,728 pixels), with the
existing per-level, tile residency and staging bounds. Beyond these limits a
named SVG resource error retains the previous display, not an unbounded raster.
The parsed tree, embedded images, renderer scratch and clipboard copies are
additional allocations, not a total-process RSS guarantee; cache accounting
continues to count straight pixels plus rendering levels. One running SVG job,
one replaceable pending target and one result prevent a queue per wheel event.
Embedded rasters do not gain vector detail when zoomed, and font appearance
still depends on installed system fonts.

## Native AVIF

AVIF (`avif`) stills use an owned header-compiled C shim over libavif, explicitly
selecting the dav1d decoder (no encoder APIs; YUV conversion avoids libyuv).
8/10/12-bit sources become full-range straight RGBA U16 and follow ordinary
ICC normalization, U16 levels, transforms and clipboard conversion. Container
rotation/mirror is applied once, with EXIF only as a fallback when neither is
present. ICC is preferred; unsupported CICP RGB primaries/transfer combinations
are disclosed as an sRGB assumption. Clean aperture (`clap`) uses libavif's
validated crop-rectangle API after YUV→RGBA conversion, including odd offsets
in subsampled images. Pixel aspect (`pasp`) is corrected to square pixels after
color normalization with linear-light premultiplied triangle resampling, then
restored to straight U16: height is retained and width
is rounded to the nearest `width × hSpacing / vSpacing` (minimum one pixel).
Crop and aspect correction precede the single rotation/mirror or EXIF fallback;
Copy Image and displayed dimensions use that corrected image. Corrected output
must still fit the 16,384-pixel-side / 512 MiB envelope, including each float
resampling working buffer (16 bytes/pixel); oversized aspect
expansions fail by name rather than silently changing proportions. Invalid
apertures that libavif cannot express as an integer crop fail by name. PQ/HLG
HDR tone mapping and sequences remain named unsupported variants. Native
decoding is in-process, not sandboxed.

The three original analytic alpha fixtures are generated from `alpha.png` with
avifenc 1.4.2, lossless, 4:4:4 identity matrix, depths 8/10/12; encoder tools are
fixture-generation tools only, never linked by Glance's adapter.
Additional original fixtures exercise 2:1 pixel aspect (5×3 → 10×3), a 10-bit
4:2:2 odd-offset crop (5×3 → 3×2), and crop/aspect/rotation/mirror with conflicting
EXIF orientation (5×3 → 2×6). PQ, HLG and two-frame sequence fixtures exercise
remaining named rejections. All derive from the same original `alpha.png`; see
[fixture generation commands](../assets/test/web/README.md) for exact generation commands. The external hato
conformance sample is used read-only for local smoke, not redistributed.
