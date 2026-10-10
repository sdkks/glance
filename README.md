# glance

An independently authored MIT desktop image viewer, built with Rust and
eframe/egui's wgpu renderer. This first increment opens **JPEG (`jpg`, `jpeg`)
and PNG**; it is not yet the complete planned viewer.

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
indicator. Default is 1; this is session-only and writes no configuration. Once a
selection settles, up to N images ahead and behind are decoded into the same
oriented RGBA8 source and rendering levels used by foreground loads. Prefetch stays
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
result is discarded, never admitted. No neighboring animation runs; this version
still only dispatches JPEG and static PNG.

Before navigation/sort, the foreground worker rescans and checks path, size,
modification time and Unix device/inode/change time. Removed or changed entries
are released; valid path-keyed entries survive sorting, while neighbors are
replanned in the new order. A file changing during decode produces a retryable
error. Metadata identity is not an atomic filesystem snapshot. Decreasing depth
trims immediately and replans; opening another folder clears neighbors/history.
Interactive latency and menu/input feel still need human desktop verification.

## Clipboard and desktop actions

File/context menus offer **Copy Image**, **Copy File**, **Paste File**, and
**Reveal in File Manager**. Like the other menu actions, these are keyboard
accessible with Tab/Shift+Tab and Enter/Space. No Cmd/Ctrl+V shortcut is advertised:
eframe's input adapter consumes that key for text-only clipboard events, which
would not reliably invoke native file-list Paste. Copy Image copies the complete decoded
image with embedded orientation, current user transforms, and straight alpha,
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

## Current limits and remaining work

Input reads are bounded at 64 MiB. Decoder output and RGBA8 output are each bounded
at 512 MiB; sides are limited to 16384 pixels, **independently of the GPU's
single-texture limit**. `image::Limits` explicitly bounds dimensions and requests a
512 MiB decoder allocation cap; the allocation cap is best-effort upstream,
not a total process-RSS guarantee. Input, decode/conversion, UI texture staging,
and GPU/native allocations may coexist. The displayed resident retains its
oriented straight CPU RGBA8 pixels for Copy Image alongside rendering levels.
Transitions retain the previous image while preparing the next visible tile set,
not a total 512 MiB budget. Copy Image allocates another full RGBA8 bitmap, checked against the
same 512 MiB per-buffer limit with a fallible allocation; native clipboard encoding
may allocate further buffers. Failure is named, never a reduced-size copy. Two workers with bounded pending/result
slots prevent growth per keypress; the resident cache envelope is described above.
These calibration values were raised after real 38MP content was rejected.

## Rendering: minification and full detail

Loader workers generate a dyadic pyramid after embedded orientation/RGBA8 conversion,
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
separately for Copy Image. This is still an 8-bit presentation boundary, not an
ICC/high-depth normalization implementation. Checker averages have a declared
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
are bounded to **16 MiB**: each at-most-4-MiB upload is submitted and drained before
the next. Device waits have a five-second failure timeout. Creation/upload validation,
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
hold one such payload. Encoded input (64 MiB per worker), decoder/conversion scratch,
clipboard output and backend allocations are separate transient costs. These are
conservative application payload bounds, not a guaranteed total RSS ceiling.

Embedded orientation is applied once. Output encoding follows the actual attachment
format (hardware sRGB encoding or explicit OETF, never both). Deliberate ICC/SDR
conversion and high-depth presentation are **not implemented yet**. Other formats
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
./scripts/install-hooks.sh
# Review the validation result, then activate manually:
git config --local core.hooksPath .githooks
```

The helper validates the tracked hook and refuses a conflicting effective
`core.hooksPath` or an existing default pre-commit hook. **It never writes Git
configuration or installs/overwrites a hook.** The tracked hook executes
`make precommit`. Activation is optional and must be done by you.

**Partial-staging caveat:** staged whitespace is checked, but formatting and
compilation use the whole working tree. This does not compile an isolated
partially staged snapshot. Run `make verify` against the actual complete tree
before review. The local hook is intentionally lighter than full verification.

## Licensing

Glance is MIT; see `LICENSE`. Direct dependencies are pinned to eframe 0.36.2
(MIT OR Apache-2.0), image 0.25.10 (MIT OR Apache-2.0), rfd 0.17.2 (MIT), arboard
3.6.1 (MIT OR Apache-2.0), and the macOS-only objc2 0.6.5 (MIT), objc2-app-kit 0.3.2
(Zlib OR Apache-2.0 OR MIT), and objc2-foundation 0.3.2 (MIT).
The viewer directly enables only image's JPEG/PNG codecs, but eframe's transitive
clipboard integration also enables TIFF in the resolved image graph; the viewer
does not dispatch TIFF. egui's bundled default fonts have their own notices.
See `THIRD_PARTY.md` for the resolved crate/license inventory and distribution
limitations. No third-party library source has been modified.
