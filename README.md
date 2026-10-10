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

Close the viewer with **⌘W on macOS / Ctrl+W on Linux**, or the window's close
button. Glance has one window, so closing it exits the app through the same
normal shutdown path. The shortcut is suppressed (and not consumed) while a
menu, popup, text control, or native file dialog owns input, or the viewer is
unfocused. **⌘Q on macOS / Ctrl+Q on Linux** remains the pinned egui stack's
built-in Quit shortcut (macOS also has its native application-menu Quit action).
Glance does not add or override that binding; unlike the viewer's guarded close
shortcut, the stack's Quit shortcut is global.

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

## Native AVIF development builds

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
artifact**. The release payload below replaces it with unmodified upstream
libavif built with dav1d only; it excludes external libyuv, encoders, libvmaf and
libheif, but retains upstream's embedded BSD-3-Clause scaling subset.
Linux release bundling remains unimplemented.
The three original analytic alpha fixtures are generated from `alpha.png` with
avifenc 1.4.2, lossless, 4:4:4 identity matrix, depths 8/10/12; encoder tools are
fixture-generation tools only, never linked by Glance's adapter.
Additional original fixtures exercise 2:1 pixel aspect (5×3 → 10×3), a 10-bit
4:2:2 odd-offset crop (5×3 → 3×2), and crop/aspect/rotation/mirror with conflicting
EXIF orientation (5×3 → 2×6). PQ, HLG and two-frame sequence fixtures exercise
remaining named rejections. All derive from the same original `alpha.png`; see
`assets/test/web/README.md` for exact generation commands. The external hato
conformance sample is used read-only for local smoke, not redistributed.

## Native AVIF release payload (macOS)

```sh
# In addition to the pinned Rust toolchain and Apple command-line tools:
brew install cmake pkg-config
python3 -m pip install --user meson ninja
make bundle-avif
# For a subsequent run, use a new, nonexistent staging directory:
make bundle-avif AVIF_STAGE=build/avif-release-2
```

The script downloads checksum-pinned libavif 1.4.2 and dav1d 1.5.4 sources
and builds both unmodified at a macOS **11.0** minimum. dav1d uses Meson
with `-Ddefault_library=shared`, `-Dc_args=-mmacosx-version-min=11.0` and
`-Dc_link_args=-mmacosx-version-min=11.0`; libavif uses CMake's explicit
`CMAKE_OSX_DEPLOYMENT_TARGET=11.0` and only the dav1d backend. Cargo also
builds with `MACOSX_DEPLOYMENT_TARGET=11.0`. Verified build prerequisites:
Cargo 1.97.1 (pinned Rust toolchain), CMake 3.29.0, Meson 1.12.1 (invoked as
`python3 -m mesonbuild.mesonmain`), Ninja 1.13.2.git.kitware.jobserver-pipe-1,
pkg-config and Apple command-line tools. Source tarball SHA-256 values are
recorded in `THIRD_PARTY.md` and verified before extraction. No external
libyuv, AOM, encoder, libvmaf or libheif library is shipped. Upstream's own five
embedded libyuv scaling source files are still compiled with `AVIF_LIBYUV=OFF`;
their BSD-3-Clause notice is in the copied upstream LICENSE. See `THIRD_PARTY.md`
for exact source hashes and provenance. Versions are checked, not upgraded.

The stage must not already exist: reruns fail rather than overwrite caller
files. `AVIF_STAGE` may be relative to the repository or absolute. A failed run
leaves its stage for diagnosis; choose another stage to retry. Downloads, native
builds and evidence under `build/` are ignored, never committed. The stage is
not a finished `.app` or signed/notarized distribution. Its contract is:

- `Contents/MacOS/glance`: release executable linked to the bundled libavif.
- `Contents/Frameworks/{libavif.16,libdav1d.7}.dylib`: only these two native
  decoder libraries; install names use executable/loader-relative paths.
- `Contents/Resources/licenses/native/`: complete upstream libavif LICENSE
  (including the embedded scaling notice), dav1d COPYING, embedded authors/source
  hashes, and verified native source archive hashes.
- Evidence beside `Contents/`: `before.txt`, `after.txt`, `smoke.txt`, `dyld.txt`,
  `sizes.txt`, `versions.txt`, `minos.txt`, payload SHA-256 files and `native-avif.zip`.
  `native-build/` and this evidence are not app contents.

`make macos-artifacts` copies **all three Contents subdirectories**, preserving
their relative locations, into the final app before signing and archiving.
The native stage script ad-hoc
re-signs rewritten Mach-O files; that is not Developer ID signing/notarization.
It audits the full dylib closure, rejecting unexpected non-system dependencies,
and traces a real packaged-binary AVIF decode with empty dyld library/fallback
paths and injected/alternate framework paths cleared. A trace that loads any
Homebrew path fails. This is relocation evidence, not a test on a clean OS install.

Measured macOS arm64 payload: libavif **269,664 B**, dav1d **924,992 B**,
combined **1,194,656 B**; dylibs plus native notices zipped **504,854 B**.
Including executable relocation/re-signing, the previously measured executable
plus dylib delta was ≈**+1.0 MB**. The native compressed increment is far under
the **40 MB compressed-DMG budget**, not gigantic. The complete AVIF-enabled
DMG is now measured at ≈**11.90 MB**, including crate/font/native notices;
see Packaging (macOS) for the complete comparison.

### Packaging/CI decoder smoke diagnostic

`GLANCE_SMOKE_IMAGE=1` decodes the first CLI image path with the real bounded
loader, prints `decoded WIDTHxHEIGHT`, and exits without opening a window. A
missing path or decoding error prints a diagnostic and exits with status 1.
Other values or an unset variable retain the unchanged normal viewer startup.
This is an opt-in packaging/CI smoke aid, not a separate decoder or GUI test:

```sh
GLANCE_SMOKE_IMAGE=1 build/bundle-libs/Contents/MacOS/glance assets/test/web/alpha8.avif
# Expected: decoded 5x3
```

### Linux packaging follow-up

`make bundle-avif` currently fails explicitly on Linux before creating a stage.
Linux release packaging must implement and validate an x86_64 counterpart: build
unmodified pinned libavif with dav1d only (same embedded scaling disclosure),
bundle libavif/dav1d `.so` files plus their complete notices, and use `$ORIGIN`
relative RUNPATHs for the executable **and** its transitive decoder dependency.
Audit the closure against system libraries, prohibit encoder/external-libyuv/
libvmaf/libheif dependencies, and run the real AVIF smoke with package-manager
codec paths unavailable. Translate the macOS stage contract to the chosen Linux
archive layout; do not ship this macOS payload or silently use system codecs.
That path and Linux measurements are outstanding, not claimed working here.

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
error. Native menus/document-open
integration and Linux desktop validation are later increments.
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
| `bundle-avif` | Stage macOS decoder-only AVIF payload; `AVIF_STAGE` selects a new directory |
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

The macOS artifact recipe below creates the `.app` bundle; there is no developer
certificate signing or notarization pipeline.
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

## Installing on Linux (source)

With the Rust toolchain and Linux development libraries described above installed:

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

Real Linux menu/icon discovery, associations, native dependencies, and cache
refresh are **UNVERIFIED** on the macOS development machine. On Linux, run:

```sh
make verify && make audit && make machete
sh scripts/test-install-linux.sh  # scratch-prefix install/uninstall isolation test
make install-linux DRY_RUN=1
make install-linux
command -v glance
ldd "$HOME/.local/bin/glance"
desktop-file-validate "$HOME/.local/share/applications/glance.desktop" # if available
# Log out/in if needed; launch Glance from the menu and use Open With on a PNG.
# Confirm the icon and image opening; this does not change your default viewer.
make uninstall-linux
```

For the standalone scratch test, `scratch=$(mktemp -d)` then
`scratch=$(cd "$scratch" && pwd -P)`,
`make install-linux PREFIX="$scratch/prefix"` and
`make uninstall-linux PREFIX="$scratch/prefix"` stage nothing in your home.
The automated script additionally checks unrelated sentinel files, modification
refusal on uninstall and reinstall, unchanged reinstall, a missing manifest,
dry-run, system-prefix guards, and DESTDIR staging with symlink-ancestor refusal.
It prints its temporary path and the exact file lists and retains the scratch
directory for inspection. The macOS scratch binary is not a Linux executable.

## Releasing

`Cargo.toml`'s `[package] version` is the single source of truth. These local
commands never push, publish crates, or create a GitHub Release:

```sh
make version
make bump-version BUMP=patch  # also minor or major; patch is the default
# Alternatively: make bump-version VERSION=0.2.0 (not together with BUMP)
git diff -- Cargo.toml Cargo.lock CHANGELOG.md
make release VERSION=0.1.1   # use the version you just reviewed; no second bump
# Review the release commit and annotated tag, then push explicitly:
git push origin <branch> v0.1.1
```

For a one-shot release, `make release BUMP=patch` bumps, commits exactly
`Cargo.toml`, `Cargo.lock`, and `CHANGELOG.md` with
`chore(release): release vX.Y.Z`, and creates the annotated `vX.Y.Z` tag.
It refuses unrelated tracked changes, detached HEAD, an existing tag, or an
unchanged release file. Bumps refresh the lockfile offline without rebuilding;
they require the pinned Rust toolchain and cached dependency metadata.
Notes under `[Unreleased]` become a UTC-dated release with a fresh empty
`[Unreleased]` section above. Explicit `VERSION` retries do not duplicate notes.
Failed bumps restore all three files; failed release commits or tag creation
restore those files and the original index, rolling back the local release commit
if needed. Do not run concurrent Git operations during release.

Pushing the reviewed branch and `vX.Y.Z` tag starts `.github/workflows/release.yml`:
**gate → package → publish**. Both Ubuntu 24.04 and macOS gates must pass
`make verify`, `make audit`, and `make machete` before either packaging job
starts. Both packages must succeed before the release is created/updated with
`GITHUB_TOKEN`. Before any asset replacement, an existing public release is
converted to a draft; a new release is created as a draft only after a confirmed
`release not found` result from the JSON-mode lookup. Other lookup errors abort
without creating a release or changing existing assets or visibility. Rerunning a tag
replaces assets with `--clobber`. The release becomes public only after all
uploads succeed and their exact names and byte sizes match the candidate set
(including rejection of extra assets). A failed upload or verification leaves
the release hidden as a draft, even if it was previously public, until a
successful rerun or manual un-draft. Manual un-draft bypasses this verification
and can expose an incomplete asset set.
No crates.io publishing,
Developer ID signing, or notarization is performed. Ordinary pushes to `main`
and pull requests run the same gates in `.github/workflows/ci.yml`.

Gate runners install distro/Homebrew development libraries. AVIF tests run only
where libavif ≥1.4.2 and dav1d ≥1.5.4 are available; Ubuntu 24.04's older packages
correctly disable AVIF there. Packaging independently builds checksum-pinned
native decoders, bundles them, and requires the existing AVIF smoke checks.

Release attachments are compressed, never raw executables or `.app` trees:

- macOS: `Glance-<version>-<arm64|x86_64>.zip`, `.dmg`, and `.sha256`
  (one checksum file covers both zip and DMG); plus
  `Glance-<version>-macos-<arch>-notices.tar.gz` and `-notices.sha256`.
- Linux: `Glance-<version>-x86_64.AppImage` and `.AppImage.sha256` (native Ubuntu
  24.04 runner; AppImage contains compressed SquashFS); plus
  `Glance-<version>-linux-<arch>.tar.gz` (portable AppRun payload and tagged
  source for `make install-linux`), `Glance-<version>-linux-<arch>-notices.tar.gz`,
  and `Glance-<version>-linux-<arch>.sha256` covering both tarballs.
- Notices archives contain the complete notices also included inside the
  application artifacts. Checksum entries use sibling basenames: download all
  attachments into one directory before checking them.

macOS runner artifacts are host-architecture builds, not Universal. They are
**unsigned by a verified developer** (ad-hoc signed only), so downloaded apps
need the Gatekeeper steps in Packaging (macOS): drag to Applications, then
right-click → Open, or remove only the trusted app's quarantine attribute.
The publish job summary lists each attachment's exact bytes and decimal MB,
total upload size, and PASS/OVER against the <40 MB compressed-DMG target.
Exceeding that target is reported, not hidden by dropping native AVIF.

**UNVERIFIED until a real tag run:** runner availability of the pinned Rust and
native versions/build tools, resulting artifact sizes, and GitHub release-write
permissions. To exercise without publishing a production release, use a fork
or dedicated test repository with Actions enabled and push this branch there.
Choose an unused version, run the local bump/release commands above, and push
its branch and tag to that test remote. In Actions, confirm both gate matrix
entries finish before packaging starts, both packages finish before publish,
and the Release contains only the named archives and checksum files. Download
all attachments into one directory and run
`for checksum in *.sha256; do sha256sum -c "$checksum" || break; done` on Linux
(or substitute `shasum -a 256 -c "$checksum"` on macOS); inspect the publish size summary.
Follow the platform packaging smoke recipes below on downloaded assets.
For a negative check on the test repository, create another unused release
version on a scratch branch, deliberately add a failing Rust test before
cutting its tag, and push that scratch branch/tag: the gate must fail, package
and publish must be skipped, and no release/assets must be created for that tag.
Do not move an existing production tag to perform these checks.

## Packaging (macOS)

```sh
make macos-artifacts       # or make dmg; prepares native AVIF + notices first
make macos-smoke IMAGE="$PWD/assets/test/web/alpha12.avif"
```

Requires macOS, the pinned Rust toolchain, the native build prerequisites listed
above (CMake, Meson, Ninja, pkg-config), Python 3.11+ for notices, and
Apple's `ditto`, `hdiutil`, `otool`, `lipo`, `plutil` and `codesign`. These are
build-time tools, not extra runtime dependencies. Outputs live in
`target/macos-artifacts/`: `Glance.app`, `Glance-<version>-<arch>.zip`,
`Glance-<version>-<arch>.dmg` and `Glance-<version>-<arch>.sha256`.
Version comes from `[package].version` in `Cargo.toml`; architecture is the
host's `arm64` or `x86_64`, not Universal. Verify downloads with
`shasum -a 256 -c Glance-<version>-<arch>.sha256` in that directory.
The zip uses `ditto --sequesterRsrc --keepParent`; the compressed DMG contains
`Glance.app` and an `/Applications` symlink. Drag the app to Applications.
The recipe is reproducible, but archive bytes/checksums may change with tool
versions or filesystem timestamps; bit-for-bit reproducibility is not claimed.

The bundle identifier is `io.github.glance-viewer.glance` (a stable application
namespace, not a claim of domain ownership). Display name is Glance, executable
is `glance`, and the icon is `Contents/Resources/glance.icns`. Both bundle version
keys use the Cargo version. The declared and compiled minimum macOS version is
**11.0**, the first Apple Silicon macOS and a common baseline for Intel/ARM;
actual oldest-OS compatibility remains unverified. Imported image UTIs cover
all eligible extensions in `src/formats.rs` (including flattened documents),
with role **Viewer** and handler rank **Alternate**, never Owner/Default.
Registration does not change existing default associations. Native Finder
file-open event handling is still deferred: launch with a CLI image argument
or use Open/drop inside the app; registration alone does not implement events.

Native AVIF is bundled **by default**: `make macos-artifacts` runs `make
bundle-avif` when `AVIF_STAGE` (default `build/bundle-libs`) does not exist,
then `make notices` using that stage's native notices. An existing stage is an
explicit reusable input; choose a fresh `AVIF_STAGE` after source/toolchain
changes to rebuild it. Partial/missing stage inputs fail loudly, never silently
produce an AVIF-less release. Direct `sh scripts/package-macos.sh [stage]`
consumes an already prepared stage and notices; it does not prepare them.

Packaging copies the stage's native-linked executable and exactly
`libavif.16.dylib` + `libdav1d.7.dylib` into `Contents/MacOS` and
`Contents/Frameworks`, preserving executable/loader-relative install names.
It checks host architecture and the complete native dependency closure, rejecting
external non-system paths. AVIF therefore works without Homebrew at runtime;
the smoke provides loader-trace evidence, not certification on a clean OS install.
Crate/font/MIT notices from `build/notices` (`NOTICES_STAGE` overrides it) go in
`Contents/Resources/licenses/`, with the native tree merged once beneath
`native/`. Conflicting duplicate notice bytes are rejected before signing.
No native-build sources, fixture assets or build logs enter the app.

Escape hatches for local, source-only bundles:

```sh
make macos-artifacts SKIP_AVIF=1 SKIP_NOTICES=1
# Or keep crate/font notices, but disable native AVIF (use fresh notice output):
make macos-artifacts SKIP_AVIF=1 NOTICES_STAGE=build/notices-source-only
```

`SKIP_AVIF=1` builds with `--no-default-features`; it does not ship a binary
linked to Homebrew. `SKIP_NOTICES=1` omits generated crate/font notices, but still
ships native license files if AVIF is enabled. **Skipping notices is not a
release-ready distribution.** With AVIF skipped, smoke still checks signatures
and GUI display of a non-AVIF image, but explicitly omits AVIF decode evidence.
`MACOS_OUT` selects a comparison output directory. Repackaging preserves the
previous direct app in `target/macos-artifacts/previous.*/Glance.app` rather than
merging stale files; archives are replaced. Retained previous apps are not shipped.

Measured arm64 0.1.0 artifacts (decimal MB; .app is summed file payload, not
filesystem allocation):

| Payload | .app | zip | DMG |
| --- | ---: | ---: | ---: |
| Native AVIF + all notices | 28.53 MB | 13.41 MB | 11.90 MB |
| No native AVIF, crate/font notices | 27.25 MB | 12.88 MB | 11.39 MB |
| Source-only, no notices | 23.80 MB | 10.94 MB | 11.08 MB |

The native AVIF + native-notices increment is ≈0.51 MB compressed DMG; the
complete DMG is **well below 40 MB**. DMG uses UDZO with `zlib-level=9`;
zip uses ditto's default compression. Sizes vary with toolchain/archive metadata.
Both source-built dylibs and the executable report Mach-O `minos 11.0`.
Bundling checks both libraries before relocation; packaging rejects every
executable/dylib with a missing deployment version or a minimum above 11.0
before signing. This prevents shipping a newer-floor bottle accidentally.
Actual runtime behaviour on macOS 11–15 and Intel remains **UNVERIFIED**;
loader metadata and current arm64 smoke are not older-OS runtime testing.

These builds are **ad-hoc signed, but not Developer ID signed or notarized**.
Packaging signs the completed bundle before archiving and verifies its signature;
`ditto` preserves that signature in the zip. Missing `codesign` or signing failure
stops packaging unless you explicitly set `ALLOW_UNSIGNED=1`, which prints a
warning and permits artifacts without a verified signature. The smoke check still
requires valid signatures.

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

The smoke command checks checksums, plist syntax, version/icon/executable keys
and all Viewer/Alternate ranks, verifies the direct app's signature, extracts the
zip, verifies its preserved signature and compares its main bundle files and
complete notices tree. For zip, direct app and mounted DMG it first decodes
`AVIF_IMAGE` (default `assets/test/web/alpha12.avif`) with empty DYLD library and
fallback paths, cleared injected/alternate framework paths and loader tracing;
it requires both bundled dylib paths and rejects Homebrew/Cellar loads.
Then it launches the GUI under the same sanitized loader environment with `IMAGE`.
It mounts the DMG read-only in a unique temporary mountpoint, verifies its app's
signature and complete notices tree,
checks the Applications link, launches its binary and requires the existing
`Displayed` diagnostic within 30 seconds, then terminates only those processes
and unmounts/removes the mountpoint. Complete logs remain in the printed temporary
evidence directory. This confirms renderer admission, not a human Dock/icon or
Gatekeeper/Finder visual check. **EXR/HDR and PQ/HLG are not supported yet.**
## Packaging notices

Run `make notices` before archiving a release. Cargo and **Python 3.11+**
(`tomllib`, standard library only) are build-time tools; Python is not shipped or
required by the viewer. The generator runs `cargo metadata --locked` with no
platform filter and reads the license allowlist and exact-version exceptions
from `deny.toml`. SPDX `AND`/`OR`/`WITH` and Cargo's legacy `/` alternatives are
checked; an expression without an allowed branch fails before output is written.
`make audit` remains the authoritative advisory/license audit, including license
text scanning; notice generation is not legal clearance.

`build/notices/` contains `THIRD-PARTY-NOTICES.md`, Glance's MIT `LICENSE`,
verbatim per-crate license/copyright/notice files (retaining distinct copyright
holders even for the same license), bundled-font notices, artwork provenance,
and the unsupported-format statement. Registry manifest paths supplied by Cargo
locate the installed crate sources, including custom `CARGO_HOME` locations.
Missing crate license files are explicitly flagged for upstream review; text is
never invented. Only notices/license texts are copied, not dependency source.

The native AVIF bundler stages libavif, dav1d and the embedded upstream scaling
subset's exact notices at
`build/bundle-libs/Contents/Resources/licenses/native/`; generation copies that
directory and its adjacent bundle `versions.txt` when present. Otherwise the
notice file contains a clear TODO: **native notices are not yet staged**. That
TODO is not release-ready: stage native notices first, then regenerate. macOS
packaging now does this automatically and includes the entire generated tree
under `Contents/Resources/licenses` before signing, so .app, zip and DMG each
carry the notices. Linux builders likewise must include the entire generated
directory in AppImage and tarball artifacts, not merely a separate attachment.

Output ordering and contents contain no generated timestamps or machine-specific
crate paths. Repeating generation with identical inputs preserves bytes and
mtimes. If the graph loses a crate/file, generation refuses stale output rather
than deleting caller files: select a fresh staging directory. Overrides:

```sh
make notices NOTICES_ARGS='--output build/notices-release --native-notices build/native-notices'
# Offline test input only (normal release generation always obtains fresh metadata):
python3 scripts/generate-notices.py --metadata /tmp/cargo-metadata.json --output build/notices-test
python3 -m unittest discover -s scripts/tests -p 'test_notices.py'
```

EXR/HDR, PQ/HLG, XCF and AVIF/JXL sequences remain unsupported.

## Linux AppImage packaging

Build **on Ubuntu 24.04 or a compatible native Linux host**, x86_64 or aarch64
(no cross-compilation). The pinned Rust toolchain must already be installed.
The build downloads unmodified libavif 1.4.2 and dav1d 1.5.4 sources, verifies
SHA-256, and builds **only dav1d decoding**, with no external libyuv, AOM,
encoders, libvmaf or libheif. Upstream libavif's own embedded BSD-3 scaling
subset is retained and attributed. No distro AVIF library is bundled.

```sh
sudo apt-get update
sudo apt-get install -y --no-install-recommends build-essential ca-certificates \
  curl cmake meson ninja-build pkg-config nasm patchelf desktop-file-utils \
  libfontconfig1-dev libxcb1-dev libxkbcommon-dev libwayland-dev libdbus-1-dev
make appimage
# Output: build/Glance-<version>-<x86_64|aarch64>.AppImage and .sha256
cd build
sha256sum -c Glance-0.1.0-aarch64.AppImage.sha256 # adapt version/architecture
chmod +x Glance-0.1.0-aarch64.AppImage
./Glance-0.1.0-aarch64.AppImage /absolute/path/to/image.avif
```

The version comes from `Cargo.toml`. Packaging pins appimagetool 1.9.1 and
**type2-runtime 20251108** separately, with architecture-specific checksums;
there is no mutable `continuous` runtime download. The AppDir contains `AppRun`,
`usr/bin/glance`, two native `.so` files in `usr/lib`, `$ORIGIN` RUNPATHs,
`usr/share/applications/glance.desktop` (`Icon=glance`, `Exec=glance %f`), and
all seven committed hicolor icons. `AppRun` preserves CLI arguments and the
caller's working directory, so relative image paths work as well.

Native licenses, source hashes, embedded-source authors and hashes, and the
AppImage runtime license/provenance are included under `usr/share/glance/licenses`.
When `scripts/generate-notices.py` is present, the complete Rust/font/native
notices bundle is generated under `usr/share/glance/notices`. Otherwise the build
warns: **complete Rust/font notices are a release prerequisite**, not included
by this standalone increment. The separate AppImage runtime embeds musl,
libfuse (LGPL), squashfuse, zstd and zlib; its license lists those terms and
upstream sources. Release distributors must satisfy those runtime obligations
as well as Glance's dependency notices; Glance itself remains MIT.

Build caches and evidence are kept under `build/appimage-<arch>`; serial reruns
refresh the same payload and artifact without stray temporary directories.
`SOURCE_DATE_EPOCH` defaults to 0 to normalize SquashFS timestamps. Reproduction
requires the same source, Cargo lock, Rust toolchain, native build tools and OS;
this is not a promise of byte identity across compilers/OS versions. AVIF build
parallelism defaults to two jobs (`JOBS=4 make appimage` overrides it).

The AppImage deliberately uses the system's glibc/libgcc, X11/Wayland, Vulkan/
Mesa drivers, fonts and desktop services; it does not bundle GPU drivers or a
whole Linux distribution. Ubuntu 24.04 is the baseline, not arbitrary older
Linux. Normal AppImage mounting needs working FUSE (`/dev/fuse`). Older runtimes
may require `libfuse2` (`libfuse2t64` on Ubuntu 24.04); this pinned runtime embeds
libfuse, so that package is not normally required. In containers or without
FUSE, use `--appimage-extract-and-run` or extract and run `squashfs-root/AppRun`.
AppImage desktop integration is not automatic: register it with your desktop's
AppImage integration tool or choose it via **Open With**. The embedded desktop
entry is metadata, not an installation into your host's application database.

### Linux verification recipe

On a native Linux host after `make appimage`:

```sh
image="$PWD/build/Glance-0.1.0-$(uname -m).AppImage" # adapt the Cargo version
fixture="$PWD/assets/test/web/alpha8.avif"
mkdir -p build/appimage-check
cd build/appimage-check
"$image" --appimage-extract
GLANCE_SMOKE_IMAGE=1 "$image" --appimage-extract-and-run "$fixture"
GLANCE_SMOKE_IMAGE=1 squashfs-root/AppRun "$fixture"
ldd squashfs-root/usr/bin/glance
ldd squashfs-root/usr/lib/libavif.so.16
ldd squashfs-root/usr/lib/libdav1d.so.7
# libavif/dav1d must resolve INSIDE squashfs-root, never /usr/lib or /opt.
desktop-file-validate squashfs-root/usr/share/applications/glance.desktop
find squashfs-root/usr/share/icons/hicolor -name glance.png
```

Both smoke invocations must print `decoded 5x3`. The diagnostic uses the real
bounded decoder and exits before creating a GUI; it is not a visual desktop test.
For container verification, mount this checkout and run the same commands:

```sh
podman run --rm -v "$PWD:/src:Z" -w /src ubuntu:24.04 bash
# Inside: install the prerequisites above, install Rust 1.97.1, make appimage,
# then execute the extraction/ldd/desktop checks (no FUSE needed).
```

Executed here on Podman 6.0.0's native arm64 VM / Ubuntu 24.04: source-native
build, AppImage extraction and extract-and-run AVIF decode, decoder-only closure,
and desktop/icon layout. **UNVERIFIED**: x86_64 runtime/build, real Ubuntu desktop
launch, file-manager Open With, folder-wide next/previous and GPU/driver behavior.
On a stock Ubuntu desktop run `./Glance-<version>-<arch>.AppImage <file>`, open a
folder, use Left/Right and Home/End, and open an image via the file manager;
confirm the icon, title/count, neighboring files and AVIF rendering.

EXR/Radiance HDR, PQ/HLG, XCF, AVIF/JXL sequences and Windows remain unsupported;
packaging does not enable those formats or HDR output.

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
