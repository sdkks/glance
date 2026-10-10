# glance

An independently authored MIT desktop image viewer, built with Rust and
eframe/egui's wgpu renderer. Opens the raster and icon formats listed below;
it is not yet the complete planned viewer.

```sh
make run ARGS='/absolute/path/to/image.png'
# Or:
cargo run --locked -- /absolute/path/to/folder
make build
./target/release/glance /absolute/path/to/image.png
```

Open with ⌘O on macOS / Ctrl+O on Linux, **File → Open…**, the **Open…** button
in empty/loading/error states, or the canvas's right-click menu. Files and folders
can also be dropped onto the window. A directory opens its first eligible image.
Folder browsing is nonrecursive and skips dot-prefixed/Finder-hidden names.
Explicitly opened hidden or missing files remain selected on opening. Corrupt
files stay in the list and show a named error; navigation and Open remain available.

Left/Right select previous/next; Home/End select first/last, without wrapping.
The **Navigate → Sort** submenu offers natural Name A→Z/Z→A, Size largest/smallest
first, and Modified newest/oldest first, indicating the active choice. Sort
persists for this session, including across Open. Before every navigation or sort,
a worker rescans the folder and reads current metadata. Sorting preserves the
selected path, not its index. If the current file disappears, navigation uses the
old order to find a surviving neighbor in the requested direction (then the other
direction at a boundary). New files join the snapshot; missing files leave it.
Natural names use ASCII case folding and numeric-run comparison (`image2` before
`image10`), then exact-name ties; metadata ties use this same ascending name order
in either direction. Unavailable metadata sorts before known values in ascending
metadata order and after known values in descending order.

Images start centered at Fit without enlarging small images. **View → Fit** (`0`
or `1`) recenters; **Actual Size (100%)** (`2`) displays one source pixel per
physical screen pixel, including HiDPI. `+` (or `=`) and `−` zoom about the canvas
center. A line-unit mouse wheel zooms at the pointer, clamped to 0.01×–500×.
Drag or two-axis point-unit trackpad scrolling pans when the image exceeds the
viewport; scrolling never navigates. Ctrl/Command+scroll also zooms at the pointer.
Hold **Space** for temporary pan mode, then **move the mouse without pressing a
button** over an enlarged image: the canvas cursor is a grab hand while held and
a grabbing hand while moving or dragging. Left-drag also pans, with or without
Space; combining them applies the pointer movement only once.
Releasing Space stops this temporary motion immediately; releasing mid-drag stops
panning until the mouse button is released. Losing
window focus or opening a menu/dialog cancels the mode. Right-click remains the
context menu; middle-click is unchanged. Space on a fitted image does nothing.
A hold may wait through loading and activates once an enlarged image is displayed.
Space has no navigation/zoom binding and is not consumed by pan mode: keyboard
controls (including future typing contexts) retain precedence. Any future explicit
Space command should take precedence over this temporary mode and suppress it.
Fit preserves the current user orientation. `R` rotates clockwise, `Shift+R`
rotates counterclockwise, `M` mirrors horizontally, and `F` flips vertically.
These View/context-menu actions compose in command order in the displayed axes,
without changing the original file. Quarter-turns/reflections only remap axes and
corners (no resampling of source pixels); ordinary viewport scaling still uses
the renderer's sampler. **Reset View** removes user transforms, restores the
already-decoded embedded orientation, and returns to Fit. Zoom/pan are applied
after image orientation; Original Size remains one transformed pixel per physical
screen pixel.

All view state resets on a different-file swap when the next image is ready to
display, not on selection. A same-path reload preserves user transforms, zoom,
and pan; there is no per-file history across navigation. During decode/upload the
prior pixels and framing remain visible;
the title marks the selected file Loading, or names its error if loading fails.
A failed load keeps the prior image visible and navigation available. Menus are keyboard-accessible
with Tab/Shift+Tab to focus and Enter/Space to activate; viewer shortcuts are
suppressed while menus or another control own focus.

The title reports filename and position/count. A foreground worker holds
one replaceable pending selection; only the current generation may be displayed.
A separate bounded worker decodes neighbors, so even an uncancellable neighbor
codec cannot queue-block a foreground request. Settled images have no application
repaint timer.

## Preloading and resident history

**Settings → Preload depth** offers Off / 1 / 2 / 3 / 4 / 5, with an active radio
indicator. Default is 1; the choice is saved across launches (see Settings below). Once a
selection settles, up to N images ahead and behind are decoded into the same
oriented straight U8/U16 source and rendering levels used by foreground loads. Prefetch stays
CPU-only until admission: a hit skips decoding but stages visible tiles through
the ordinary atomic swap. Up to N recently admitted images retain CPU sources
and levels, **not inactive GPU textures**. History hits therefore skip decode and
level generation, but reupload visible tiles. View transforms still reset on a
different-file admission; history does not preserve per-file view state.
Off stops prefetch and releases cached entries immediately.

At most 2N cached entries plus the displayed image are retained (history and
neighbors share those slots). Their combined accounted CPU byte budget is
**max(512 MiB, twice the largest image admitted in this folder)**. The formula
is unchanged; an entry now counts its straight source **plus every retained
rendering level, including the premultiplied base**, rather than the source alone.
Oldest history is evicted first, then oldest prefetch; the displayed image is never
an eviction candidate. History recency changes on admission, not background
decoding. Shared immutable payloads count once; history entries have no GPU tiles
to account. GPU residency and transient gutter buffers are bounded separately below.

One in-flight decode/result per worker, encoded input, decoder/conversion scratch,
GPU staging and clipboard copies are **additional bounded/transient allocations**,
not included in the resident cache budget and not a process-RSS guarantee. A
running synchronous prefetch codec may finish after cancellation; its stale
result is discarded, never admitted. No neighboring animation runs: GIF/APNG/WebP
prefetch decodes only the first composed frame. Animation playback is created only
after foreground display admission. Played animation frames are not retained in
history (reopening starts from the first frame); TIFF still uses its first page.

Before navigation/sort, the foreground worker rescans and checks path, size,
modification time and Unix device/inode/change time. Removed or changed entries
are released; valid path-keyed entries survive sorting, while neighbors are
replanned in the new order. A file changing during decode produces a retryable
error. Metadata identity is not an atomic filesystem snapshot. Decreasing depth
trims immediately and replans; opening another folder clears neighbors/history.
Interactive latency and menu/input feel still need human desktop verification.

## Animation playback

GIF, 8-bit APNG (`.png`), and animated WebP start playing when opened. The existing
`image` decoder composes disposal, offsets and transparency; Glance does not collect
an entire animation. GIF Netscape finite values count repetitions after the initial
play; APNG/WebP finite values count total plays. Infinite instructions keep playing.
A GIF without a loop instruction plays once and holds its final frame.

The **Playback** menu offers Pause/Resume (`P`) and Next Frame (`.`). Pause holds the
displayed composed frame; Resume continues its timer. Next Frame pauses, advances
exactly one composed frame from the displayed frame, and wraps for inspection.
These shortcuts respect typing/menu/window focus. Space remains temporary pan.
A new file starts playing unless Animation rate is Off, regardless of the previous
animation's pause state.
Rotate/mirror/flip, zoom/pan and Actual Size apply to every displayed frame; Copy
Image copies the currently displayed full composed frame with those transforms.

Browser-compatible timing maps missing/zero delays and encoded delays ≤10 ms
(0–1 GIF centiseconds) to 100 ms. Other delays have a default 20 ms floor
(maximum 50 fps): 0, 1, 2, 5, 10 and 20 centiseconds play as 100, 100, 20, 50,
100 and 200 ms respectively. Delays ≥20 ms, including fractional durations, are
honored as encoded. **Settings → Animation rate** offers these persisted choices,
with a radio indicator for the active mode:

- **Honor file (max 50 fps)** (default): browser-compatible timing described above.
- **Honor file exactly**: encoded delays, with only a 5 ms safety floor (no 50 fps cap).
- **30 fps**, **24 fps**, **15 fps**, **10 fps**: fixed intervals of 1000/fps ms,
  ignoring encoded delays while preserving the file's loop structure.
- **Off (first frame only)**: a newly opened animation stays on its first composed
  frame. Switching Off during playback holds the currently displayed frame,
  without restarting the file. **Next Frame** still steps manually; Resume does
  not override Off.

A rate change applies immediately without reload or position reset: the next
interval starts at the change using the displayed frame's delay and new policy.
An explicitly paused animation stays paused. Timing uses a monotonic clock and
cumulative deadlines, not `now + delay` after every late decode. Under load, late
frames can be omitted from presentation, but their dependents are decoded in order.
GPU coverage is admitted atomically; a decoding/upload error retains the prior frame.
Manual stepping may replay composition from the beginning, so it can take longer
in a very long animation. No seeking, speed multipliers, editing or slideshow is provided.

The pull-driven pipeline retains the displayed frame plus at most two normalized
lookahead/candidate frames, never a growing list across loops. Within the 512 MiB
animation envelope, admission reserves three source-and-level payloads (each at
most three times raw RGBA8) plus three raw compositor/reference buffers. A raw
animation canvas must therefore fit 512 MiB / 12 (about 42.7 MiB); the existing
16,384-pixel side and 64 MiB encoded-input limits also apply. Encoded input, GPU
tiles/staging, cache entries, conversion scratch and clipboard copies remain
separately bounded/transient; this is not a total-process RSS guarantee. Canceled
synchronous decoder work may finish, but cannot publish into a replaced playback.
Prefetch keeps only the first composed frame; history never retains played frames
or running decoders. Changing preload depth or sort does not start neighbor playback.

AVIF and JXL sequences remain deferred; still support is described below.
16-bit APNG playback is not
supported; static 16-bit PNG is unchanged. Visual playback and pause/step feel still
require a human desktop check. For release smoke diagnostics, set
`GLANCE_TRACE_ANIMATION=1` to log each fully admitted animation frame to stderr;
normal playback does not emit per-frame logs.

## Settings persistence

Preload depth and animation rate are loaded before the first folder admission from a human-readable
TOML file:

- macOS: `$HOME/Library/Application Support/glance/settings.toml`
- Linux: `$XDG_CONFIG_HOME/glance/settings.toml` when that variable is absolute;
  otherwise `$HOME/.config/glance/settings.toml`
- Windows: persistence is deferred; settings remain session-only.

The stable key `preload-depth = 1` accepts integers 0 (Off) through 5.
`animation-rate = "honor-file"` accepts `honor-file`, `exact`, `30-fps`, `24-fps`,
`15-fps`, `10-fps`, or `off`. An unknown or invalid rate value falls back to
`honor-file` without discarding a valid preload depth. Unknown
keys are tolerated on load (not retained when saving); missing keys use defaults.
Sort still remains session-only. Missing/malformed files or invalid depth values
use depth 1 with a debug diagnostic, without a dialog. Missing or nonabsolute
HOME disables persistence, including when XDG_CONFIG_HOME is present.

Each setting change immediately updates its policy and synchronously writes a
unique temporary file in the same directory, flushes it, then atomically renames
it over the settings file. Force-quit after a completed change preserves the
choice; readers see either the old or the complete new file, never partial output.
Failed writes clean up the temporary file and disable persistence for the rest
of that session, with only a debug diagnostic; viewing and menu changes continue.
A crash before rename can leave a temporary file, ignored on subsequent launches.
Atomic replacement is not a promise of power-loss durability of directory metadata.

## Clipboard and desktop actions

File/context menus offer **Copy Image**, **Copy File**, **Paste File**, and
**Reveal in File Manager**. Like the other menu actions, these are keyboard
accessible with Tab/Shift+Tab and Enter/Space. No Cmd/Ctrl+V shortcut is advertised:
eframe's input adapter consumes that key for text-only clipboard events, which
would not reliably invoke native file-list Paste. Copy Image copies the complete decoded
image with embedded orientation, current user transforms, and straight alpha,
with color normalization applied. High-depth sources are downconverted to an
8-bit sRGB clipboard bitmap only when copying (a clipboard limitation, not a
presentation downgrade),
not a viewport crop or a zoomed preview. Copy File copies a native local file
reference separately. During a pending or failed load, copy/reveal refer to the
resident actually displayed, not the provisional selection shown in the title.
Paste opens the first local file reference via the normal Open path. Plain
absolute paths and `file://` URLs (empty authority or `localhost`) are accepted;
remote URLs, relative paths, and raw bitmap opening are rejected with a named
message. Clipboard/action errors appear in a dismissible strip without losing
the displayed image or navigation.

Clipboard uses the pinned direct arboard 3.6.1 dependency. On macOS, Copy File
writes AppKit NSURLs. A small Paste adapter uses already-resolved objc2 0.6.5,
objc2-app-kit 0.3.2, and objc2-foundation 0.3.2 to inspect full NSURL strings before
acceptance: arboard's macOS file-list reader otherwise drops URL authorities.
On Linux, arboard reads/writes `text/uri-list`, and decoded paths are validated as
absolute local paths before opening; text clipboard contents use our URL parser.
The clipboard handle stays alive while Glance runs; persistence after exit is not
promised. Linux Wayland/X11 ownership and receiving-app behavior are **UNVERIFIED**.

Reveal starts `open -R <file>` on macOS or `xdg-open <parent-directory>` on Linux,
with separate argv and no shell interpolation. Linux parent-open behavior is
**UNVERIFIED** on a real desktop. Launch/exit failures are reported; a single
pending reveal process is polled non-blockingly. Human verification is still
needed for transformed alpha pasting into Preview/other receiving applications,
native file-reference round trips, Finder selection, and transform/input feel.

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
Ordinary 8-bit images retain the encoded premultiplied RGBA8 path below.

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
rules, not a browser viewport. This increment rasterizes once at intrinsic size
(on the loader worker), keeps straight U8 plus ordinary rendering levels, and
copies that intrinsic-size bitmap. It does **not** retain the parsed tree or
re-rasterize at zoom/DPI thresholds: enlargement is a raster preview and may
pixelate. Scale-aware sharp vector zoom remains unfinished, not claimed here.

## Native AVIF development builds

AVIF (`avif`) stills use an owned header-compiled C shim over libavif, explicitly
selecting the dav1d decoder (no encoder APIs; YUV conversion avoids libyuv).
8/10/12-bit sources become full-range straight RGBA U16 and follow ordinary
ICC normalization, U16 levels, transforms and clipboard conversion. Container
rotation/mirror is applied once, with EXIF only as a fallback when neither is
present. ICC is preferred; unsupported CICP RGB primaries/transfer combinations
are disclosed as an sRGB assumption. PQ/HLG, sequences, clean aperture and
non-square pixel aspect properties are named unsupported variants, not silently
misrendered. Native decoding is in-process, not sandboxed.

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
artifact**. Release packaging must provide/pin a decoder-only libavif+dav1d build,
without encoders, libyuv, libvmaf or libheif, and collect exact native notices.
This increment does not provision that artifact. Linux remains unverified.
The three original analytic alpha fixtures are generated from `alpha.png` with
avifenc 1.4.2, lossless, 4:4:4 identity matrix, depths 8/10/12; encoder tools are
fixture-generation tools only, never linked by Glance's adapter.

## Current limits and remaining work

Input reads are bounded at 64 MiB. Decoder output and normalized straight output
are each bounded at 512 MiB; U8 RGBA uses 4 bytes/pixel, U16 uses 8, and float
working RGBA uses 16 (also checked against 512 MiB before conversion). Sides are
limited to 16384 pixels, **independently of the GPU's single-texture limit**. `image::Limits` explicitly bounds dimensions and requests a
512 MiB decoder allocation cap; the allocation cap is best-effort upstream,
not a total process-RSS guarantee. Input, decode/conversion, UI texture staging,
and GPU/native allocations may coexist. The displayed resident retains its
oriented straight CPU U8/U16 pixels for Copy Image alongside rendering levels.
Transitions retain the previous image while preparing the next visible tile set,
not a total 512 MiB budget. Copy Image allocates another full RGBA8 bitmap, checked against the
same 512 MiB per-buffer limit with a fallible allocation; native clipboard encoding
may allocate further buffers. Failure is named, never a reduced-size copy. Two workers with bounded pending/result
slots prevent growth per keypress; the resident cache envelope is described above.
These are the combined color/rendering envelope, superseding the interim
source-only calibration. A 38MP 16-bit RGBA source is about 292 MiB; its full
half-float pyramid is normally about 389 MiB, fitting the per-buffer limits.

## Rendering: minification and full detail

Loader workers generate a dyadic pyramid after embedded orientation/color normalization,
not on the UI thread. Each child side is `max(1, floor(parent_side / 2))`.
A child pixel averages the complete parent box from `floor(i × parent / child)`
to `floor((i + 1) × parent / child)` (exclusive), so odd-edge remainder texels
are never dropped. Levels are generated globally before tiling, not independently
at tile edges. All pyramid levels are retained to support arbitrary subsequent
zoom without discarding native detail; no second pyramid or speculative variants
are retained. Their total payload is bounded by twice the straight source bytes
(including the premultiplied base; normally about 4/3 for two-dimensional images).

**Premultiplication convention:** rendering RGB is premultiplied **in linear light**,
then sRGB-encoded into RGBA8; alpha is linear. Hardware sRGB texture sampling
linearizes RGB before filtering. Generation decodes the same convention before
averaging; the shader blends already-premultiplied RGB and composites over the
dark canvas without multiplying alpha a second time. The straight source is kept
separately for Copy Image. U16 sources instead use the linear half-float convention
above. Checker averages have a declared
linear-channel tolerance of 0.004 and premultiply/unpremultiply roundtrip tolerance
of 0.006 for the unit reference patterns. Human photo/shimmer/halo checks remain
necessary; these numerical checks are not a visual-quality certification.

Fit/reduction blends the two neighboring levels using logarithmic physical-pixel
scale and their actual dimension ratios (including odd and thin images). Actual
Size and enlargement use the native base with nearest sampling for pixel inspection.
The renderer queries `max_texture_dimension_2d` and uses interiors of at most
1022 pixels per side, reduced further for small device limits, plus a one-texel
filtering gutter on every side. Interior tiles partition the entire base and every
level; edge gutters clamp only at the complete image boundary. Panning reuploads
base tiles in any corner; a fit preview is never the only retained detail.

Only visible tiles from the selected levels plus at most eight adjacent tiles per
level are uploaded. GPU textures have a **128 MiB per-renderer LRU budget**;
offscreen tiles are evicted first, and the complete current visible set is protected.
A viewport whose mandatory visible set exceeds that budget reports a named resource
error rather than displaying partial detail. CPU gutter buffers and queued uploads
are bounded to **16 MiB**: each at-most-8-MiB high-depth upload (4 MiB for U8)
is submitted and drained before
the next. Creation/upload validation,
internal and allocation errors become `Display unavailable` errors. A failed load
or staging operation retains the prior image and navigation. Replacement admission
requires a complete-visible-coverage witness, never the first successful tile.

A retained prior paint callback protects its textures on pan/upload failure. Allow
up to **512 MiB of application-held GPU tile references** across current residency,
prior paint, replacement staging and the outgoing frame; history never adds GPU
residency. Driver-internal allocations and release timing are not a process-RSS
promise. The straight source plus levels is bounded by **1.5 GiB per payload**;
the dynamic resident CPU cache thus has a conservative **3 GiB** maximum. Each of
the two bounded worker slots and an awaiting-admission replacement may additionally
hold one such payload. Encoded input (64 MiB per worker), orientation/conversion
scratch (up to two additional 512 MiB image buffers per worker), transactional ICC
output (one extra normalized-source-sized buffer, at most 512 MiB), and CMS
profile/tables (4 MiB profile/CLUT caps, 65,536 TRC entries; CMS internal tables
are additional bounded-by-transform costs) can coexist transiently. U16 doubles
sample/tile bytes, not the fixed byte caps: it reduces maximum admitted pixel
count. Clipboard output is at most 512 MiB plus native encoding/backend costs.
These are conservative application payload bounds, not a guaranteed total RSS
ceiling; allocator/codec/driver allocations remain outside an exact reservation
tracker.

Embedded orientation is applied once. Output encoding follows the actual attachment
format (hardware sRGB encoding or explicit OETF, never both). Other formats
(including explicitly deferred XCF) give an unsupported-format
error. Animation, native menus/document-open
integration, packaging and Linux desktop validation are later increments.
Interactive zoom/pan feel and real input-device classification still need human
verification on target desktops. No qView code or assets are included.

## Development and quality gates

The pinned Rust toolchain is **1.97.1**, with rustfmt and Clippy. Install it yourself
before running checks; gates do not install tools, update the lockfile, or fix files.
`Cargo.lock` is committed. Linux builds need the normal windowing/graphics system
libraries and a working X11 or Wayland desktop; this increment has only been
smoke-tested on macOS arm64, not certified on Linux.

| Make target | Direct command / behavior |
| --- | --- |
| `help` (default) | List targets |
| `fmt` | `cargo fmt --all` — explicitly modifies source |
| `fmt-check` | `cargo fmt --all -- --check` |
| `check` | `cargo check --workspace --all-targets --locked` |
| `clippy` | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| `test` | `cargo test --workspace --locked` |
| `verify` | Sequential, fail-fast fmt-check → check → clippy → test, even with `make -j` |
| `icons` | Rebuild derived icon files from committed masters using macOS `sips` + `iconutil` |
| `install-hooks` | Opt in to tracked hooks using local `core.hooksPath = .githooks`; refuse conflicts |
| `uninstall-hooks` | Clear local `core.hooksPath` only when exactly one value equals `.githooks` |
| `precommit` | `git diff --cached --check`, then working-tree fmt-check and check |
| `build` | `cargo build --release --locked` |
| `run` | `cargo run --locked -- $(ARGS)` |

Diagnostics and failing exit statuses are not filtered. Checks do not format,
fix, stash, stage, clean, or overwrite hooks. Cargo will create normal ignored
build artifacts. Tests create original fixtures in uniquely named temporary
folders, not from legacy assets.

### Opt-in dependency audits

Install the tested developer-tool versions explicitly (never installed by make):

```sh
cargo install cargo-deny --version 0.20.2 --locked
cargo install cargo-machete --version 0.9.2 --locked
make audit
make machete
```

`audit` runs `cargo deny --locked check` across the resolved graph, including
platform-specific crates: license policy in `deny.toml`, security advisories and
yanked releases (no ignores), duplicate-version warnings, wildcard bans, and
crates.io-only sources. LLVM license riders are allowed only for the listed
crate versions; permissive OR alternatives do not authorize copyleft licenses.
The allowlist includes bundled font licenses; distribution still needs the
notices described in `THIRD_PARTY.md`.

`machete` runs `cargo machete` to detect unused direct dependencies. Both targets
are non-mutating with respect to tracked files and fail clearly if the tool is
missing. Cargo and cargo-deny may populate local caches or fetch advisory data;
audit results depend on the current advisory database and registry state.
They are intentionally outside `verify` and the pre-commit hook: these opt-in,
CI-tier dependency checks require extra tools and advisory/network access, while
`verify` remains fmt-check → check → clippy → test.

### Opt-in pre-commit hook

From the repository root:

```sh
make install-hooks
# To remove this local activation later:
make uninstall-hooks
```

Activation is opt-in. Both targets use `scripts/install-hooks.sh`, which resolves
the repository root. Installation checks the effective `core.hooksPath`: an
already-active `.githooks` succeeds without changes; a different nonempty value
is refused, never overridden. Inspect conflicts with
`git config --show-origin --get-all core.hooksPath`. When unset or empty, the
helper validates the executable tracked pre-commit hook and sets only the local
`core.hooksPath` to `.githooks`. No files are copied into `.git/hooks`; any default
hooks there remain untouched but are bypassed while `.githooks` is active.
Uninstallation clears the local setting only when exactly one local value exists
and equals `.githooks`, refusing duplicate, foreign, empty or unset values;
inherited global/system hooksPath settings can then apply again.

The only active tracked hook is **pre-commit**, which executes `make precommit`.
There are no commit-msg or post-commit hooks: no message convention or post-commit
check is currently required, so no empty scaffolding is installed.

**Partial-staging caveat:** staged whitespace is checked, but formatting and
compilation use the whole working tree. This does not compile an isolated
partially staged snapshot. Run `make verify` against the actual complete tree
before review. The local hook is intentionally lighter than full verification.

## App icons

Run `make icons` on macOS to regenerate `resources/glance.icns`, the ten PNGs in
`assets/icons/glance.iconset/`, and the seven Linux PNGs in
`assets/icons/hicolor/{16,24,32,48,64,128,256}x{same}/apps/glance.png`.
Only these derived files are written; `assets/icon/` is unchanged. Generation
uses a temporary iconset, system `sips` resampling and `iconutil` conversion,
and preserves destination mtimes when bytes match. No Downloads files, Python,
or runtime dependencies are required. Outputs were byte-identical across two
runs with sips-316 / macOS 26.6.2; byte identity across tool/OS versions is not
promised. No embedded timestamp differences were observed.

The source of truth is the single committed master
`assets/icon/glance-icon-master.png`, the human-supplied artwork
(`glance-enhanced.png`, 2048×2048, RGBA with a real alpha channel — the rounded
edges are transparent). It is used for **both** platforms: the macOS iconset and
the Linux hicolor set are derived from it. `make icons` asserts that every
rendered size still reports an alpha channel and refuses to publish a flattened
result, so transparency cannot be silently lost by a future tool change.

This replaces the earlier attempt that cropped a supplied artwork sheet literally
from a 1152×864 reference grid (`glance app assets.jpg` is 2400×1792). Those crops
did not land on the artwork and were rejected by the human; the crops and the
second Linux master were removed. Do not reintroduce sheet cropping.

Future macOS packaging must copy `resources/glance.icns` to
`Glance.app/Contents/Resources/glance.icns` and put this entry in
`Glance.app/Contents/Info.plist`:

```xml
<key>CFBundleIconFile</key>
<string>glance.icns</string>
```

There is currently no `.app` bundle, packaging, signing, or notarization pipeline.
Dev runs embed the 256px platform PNG using eframe's viewport `IconData`; eframe
also sets the macOS application/Dock icon. A human should confirm Dock appearance.
On Linux, future packaging should install the hicolor tree under
`/usr/share/icons/hicolor/` (or `$HOME/.local/share/icons/hicolor/` for a user
install), with `Icon=glance` in the desktop entry. Wayland/compositor discovery
may require that desktop entry; viewport icons alone do not replace packaging.

Linux cannot run the macOS-only `make icons` target: use the committed PNGs or
regenerate them on macOS. An offline Linux alternative, if updating artwork,
is ImageMagick, e.g. `magick assets/icon/glance-icon-master.png -resize
24x24 assets/icons/hicolor/24x24/apps/glance.png`, repeated for each listed size.
That alternative is not a project dependency, has not been validated here, and
need not match sips bytes; `.icns` regeneration still requires macOS.
See `THIRD_PARTY.md` for the human-supplied artwork provenance.

## Licensing

Glance is MIT; see `LICENSE`. Direct dependencies are pinned to eframe 0.36.2
(MIT OR Apache-2.0), image 0.25.10 (MIT OR Apache-2.0), rfd 0.17.2 (MIT), arboard
3.6.1 (MIT OR Apache-2.0), and the macOS-only objc2 0.6.5 (MIT), objc2-app-kit 0.3.2
(Zlib OR Apache-2.0 OR MIT), and objc2-foundation 0.3.2 (MIT).
The viewer directly enables image's JPEG/PNG/TIFF/BMP/ICO/TGA/PNM/GIF/WebP codecs
with defaults disabled, plus image-extras 0.1.1 (MIT OR Apache-2.0) with the selective
features listed above. Color conversion uses
moxcms 0.8.1 (BSD-3-Clause OR Apache-2.0); RGBA16F packing uses half 2.7.1
(MIT OR Apache-2.0), promoted from the existing resolved graph. Settings parsing
uses toml_edit 0.25.17 (MIT OR Apache-2.0), also promoted without new resolved
packages. egui's bundled
default fonts have their own notices.
See `THIRD_PARTY.md` for the resolved crate/license inventory and distribution
limitations. No third-party library source has been modified.
