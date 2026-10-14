# Rendering and limits

Rendering behavior, resource bounds and remaining validation work. See [Using Glance](usage.md) for controls and cache behavior, and [Supported formats](formats.md) for format-specific limits.

## Current limits and remaining work

The image, cache, playback and GPU residency bounds described here apply **per
window**, not to the whole app. Each window retains its own image and caches;
several windows can approach the sum of the single-window bounds. There is no
shared global memory budget. The shared GPU upload throttle still bounds
in-flight staging, but does not cap total residency across windows.

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
slots prevent growth per keypress; the resident cache envelope is described in [Preloading and resident history](usage.md#preloading-and-resident-history).
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
separately for Copy Image. U16 sources instead use the [linear half-float convention](formats.md#color-and-depth). Checker averages have a declared
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
are bounded to **16 MiB of payload**: multiple tiles are written into one batch
of at most **8 MiB** (including gutters), alongside one transient CPU gutter of
at most **8 MiB**. A maximum-size high-depth tile fills a batch; U8 and edge tiles
can share it. Only one batch may be in flight across all windows and selections,
including cancelled loads, and no second batch starts in the same UI pass.
The batch is submitted once and drained before the next batch. Coverage is
withheld until a later pass observes completion of every required tile. Partial
batch errors still submit queued writes and retain the shared staging slot until
completion, even if the originating selection is cancelled. After a successful
drain, a local allocation/create/write error is returned without preventing later
staging. Failed or uncertain submission, device loss and polling failures latch a
viewer-wide display failure; completion cannot clear that failure.
Creation/upload validation,
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
error. Native menu integration and Linux desktop validation remain unverified.
Interactive zoom/pan feel and real input-device classification still need human
verification on target desktops. See the [README](../README.md) for the
no-shared-code provenance note.
