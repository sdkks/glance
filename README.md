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

Open with ⌘O on macOS / Ctrl+O on Linux, the **Open…** button in empty/loading/error
states, or the canvas's right-click **Open…** action. Files and folders can also be
dropped onto the window. A directory opens its first eligible image. Folder
browsing is nonrecursive, skips dot-prefixed names, and sorts names naturally
(`image2` before `image10`), with ASCII case folding and exact-name tie breaks.
Explicitly opened hidden or missing files remain selected. Corrupt files stay in
the list and show a named error; navigation and Open remain available.

Left/Right select previous/next; Home/End select first/last. Navigation stops at
the ends. The title reports filename and position/count. Images are fitted and
centered without enlarging small images (source size is measured in physical
pixels). A single background worker holds one replaceable pending selection;
only the current generation may be displayed. Settled images have no application
repaint timer.

## Current limits and remaining work

Input reads are bounded at 64 MiB. Decoder output and RGBA8 output are each bounded
at 128 MiB; sides are limited to the smaller of 8192 pixels and the device's
texture limit. `image::Limits` explicitly bounds dimensions and requests a
128 MiB decoder allocation cap; the allocation cap is best-effort upstream,
not a total process-RSS guarantee. Input, decode/conversion, UI texture staging,
and GPU/native allocations may coexist. One worker and bounded pending/result
slots prevent growth per keypress. These are interim limits, not the future
full-detail large-image envelope.

Embedded orientation is applied once. Presentation currently uses an egui RGBA8
texture with linear filtering. Deliberate ICC/SDR conversion, high-depth
presentation, strong minification, and oversized-image tiling are **not implemented
yet**. Other formats (including explicitly deferred XCF) give an unsupported-format
error. Animation, zoom/pan/user transforms, clipboard/reveal, additional sorts,
folder refresh, native menus/document-open integration, packaging and Linux
desktop validation are later increments. No qView code or assets are included.

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
(MIT OR Apache-2.0), image 0.25.10 (MIT OR Apache-2.0), and rfd 0.17.2 (MIT).
The viewer directly enables only image's JPEG/PNG codecs, but eframe's transitive
clipboard integration also enables TIFF in the resolved image graph; the viewer
does not dispatch TIFF. egui's bundled default fonts have their own notices.
See `THIRD_PARTY.md` for the resolved crate/license inventory and distribution
limitations. No third-party library source has been modified.
