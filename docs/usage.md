# Using Glance

Detailed controls and behavior for image viewing. See [Installing Glance](installing.md) for setup, [Supported formats](formats.md) for codec boundaries, and [Rendering and limits](rendering.md) for resource budgets.

## Opening, navigation and view controls

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
viewport; scrolling never navigates. Ctrl/Command+scroll and trackpad pinch gestures
zoom at the pointer, using the same 0.01×–500× limits.
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

## Transparency backdrop

For an image with non-opaque pixels, press unmodified `b` to cycle the backdrop
white → grey → black → white, starting from the active choice. This override is
local to that window and image: it resets when another image is admitted (Open,
navigation, first/last, file/folder drop), not while a replacement is still loading.
Animation frames and SVG rerasterization retain the choice. On an opaque image,
`b` does nothing; the usual menu, typing and window-focus guards apply.

**Settings → Background color** selects the shared, persisted default: White,
Grey or Black. It applies to newly admitted images and after relaunch, and to
current images without a `b` override. The setting only matters for transparent
pixels; canvas and chrome stay unchanged. Black is the existing near-black
backdrop, so the default appearance is unchanged.

## Windows

Choose **File → New Window** to open an empty, independent image window, then
use Open or drop an image into it. Each window has its own image, folder
position, zoom/pan and orientation, playback position/pause state, and action
errors. Each draws its own menu bar. Settings and the clipboard are shared
across the app; changing animation rate or preload depth applies to all windows.

On macOS, **Settings → Open new files in** controls documents delivered while
Glance is running, including Finder double-clicks and `open -a Glance`:

- **Current window** (default): replaces the focused window's image. A window
  already showing that file takes priority; otherwise Glance uses the most
  recently focused live window, then the sole remaining window.
- **New window**: opens the document in a new window, leaving other windows
  untouched, even if one already shows that file.

The choice is saved across launches in the [settings file](#settings-persistence).
A request containing several documents opens only the first. Startup is unchanged:
the launch document opens in the first window regardless of this setting.
In-app Open, Paste File and dropping a file still open it in that window.

**⌘W on macOS / Ctrl+W on Linux**, or the window's close button, closes only
that window; other windows stay open. Closing the last window quits Glance.
**⌘Q on macOS / Ctrl+Q on Linux** quits the whole app from any focused window.
Both shortcuts are suppressed while a menu, popup, text control or native file
dialog owns input, or the window is unfocused. Memory use grows with the number
of open windows; see the [per-window limits](rendering.md#current-limits-and-remaining-work).

## Preloading and resident history

**Settings → Preload depth** offers Off / 1 / 2 / 3 / 4 / 5, with an active radio
indicator. Default is 1; the choice is saved across launches (see [Settings persistence](#settings-persistence)). Once a
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
to account. GPU residency and transient gutter buffers are bounded separately in [Rendering and limits](rendering.md).

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

GIF, 8-bit APNG (`.png`), and animated WebP start playing when opened. Their
[decoders](formats.md) compose disposal, offsets and transparency; Glance does not collect
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

The worker decodes/composes one raw successor (or observes EOF) before publishing
its current candidate, preserving exact final-frame detection. After publication,
it applies orientation, color normalization and rendering-level generation to that
successor while the UI can upload the current candidate. This shared GIF/APNG/WebP
path uses the same worker and lookahead slot, not another queue or decoder thread.
A preparation failure is terminal and remains available even if the current
candidate has already been taken; an untaken candidate is replaced by that error.

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

AVIF and JXL sequences remain deferred; still support is described in [Supported formats](formats.md).
16-bit APNG playback is not
supported; static 16-bit PNG is unchanged. Visual playback and pause/step feel still
require a human desktop check. For release smoke diagnostics, set
`GLANCE_TRACE_ANIMATION=1` to log each fully admitted animation frame to stderr;
normal playback does not emit per-frame logs.

## Settings persistence

Preload depth, animation rate, background color and the open-new-files choice are loaded before the
first folder admission from a human-readable TOML file:

- macOS: `$HOME/Library/Application Support/glance/settings.toml`
- Linux: `$XDG_CONFIG_HOME/glance/settings.toml` when that variable is absolute;
  otherwise `$HOME/.config/glance/settings.toml`
- Windows: persistence is deferred; settings remain session-only.

The stable key `preload-depth = 1` accepts integers 0 (Off) through 5.
`animation-rate = "honor-file"` accepts `honor-file`, `exact`, `30-fps`, `24-fps`,
`15-fps`, `10-fps`, or `off`. An unknown or invalid rate value falls back to
`honor-file` without discarding a valid preload depth.
`open-new-files-in = "current-window"` accepts `current-window` or `new-window`;
a missing, unknown or invalid value uses `current-window` without discarding the
other valid settings. `background-color = "black"` accepts `white`, `grey` or
`black`; missing, unknown or wrong-typed values use `black` without changing other
valid preferences. Unknown keys are tolerated on load (not retained when
saving); missing keys use defaults.
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
promised. Linux Wayland/X11 ownership and receiving-app behavior have not yet been verified.

Reveal starts `open -R <file>` on macOS or `xdg-open <parent-directory>` on Linux,
with separate argv and no shell interpolation. Linux parent-open behavior is
has not yet been verified on a real desktop. Launch/exit failures are reported; a single
pending reveal process is polled non-blockingly. Human verification is still
needed for transformed alpha pasting into Preview/other receiving applications,
native file-reference round trips, Finder selection, and transform/input feel.

## Making Glance the default image viewer

Choose **Settings → Set as the default image viewer…** and confirm to replace
associations for all supported image types. The result appears in a dismissible
notice; failures are reported without interrupting viewing. Nothing changes at
installation time. On macOS, run the installed `Glance.app`; the in-app action
requires macOS 12 or newer and runs in the background. A partial failure can leave
some types changed; check your system defaults before retrying.

Manual alternatives:

- macOS (including macOS 11): select an image in Finder, choose **Get Info →
  Open with → Glance → Change All…**, repeating for other types as needed. On
  macOS 12+, you can also use `duti`, for example
  `duti -s io.github.glance-viewer.glance public.png all` (install `duti` separately).
- Linux: use your desktop's **Default Applications** panel or
  `xdg-mime default glance.desktop image/png`, repeating for other MIME types.
  The in-app action updates `$XDG_CONFIG_HOME/mimeapps.list` (or
  `~/.config/mimeapps.list`), preserving unrelated entries.

An AppImage's `.desktop` file lives inside the image, not in the desktop's search
path. Use `make install-linux` or copy `resources/glance.desktop` into
`~/.local/share/applications` first, and ensure its `Exec`/`TryExec` resolve to
Glance (for an AppImage, adapt them to its installed executable path).
Real macOS association changes and Linux desktop integration have not yet been verified.
Finder document opening is supported at launch and while Glance is running;
see [Windows](#windows) for the open-new-files policy.
