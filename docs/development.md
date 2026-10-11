# Development

Contributor tools and quality checks. See [Installing Glance](installing.md#building-from-source) for prerequisites and [Packaging and releases](packaging.md) for distribution builds.

## Development and quality gates

Builds record the source commit SHA; see **Help → About Glance** to identify a binary.

See [Building from source](installing.md#building-from-source) for the pinned
toolchain and platform prerequisites.

| Make target | Direct command / behavior |
| --- | --- |
| `help` (default) | List targets |
| `fmt` | `cargo fmt --all` — explicitly modifies source |
| `fmt-check` | `cargo fmt --all -- --check` |
| `check` | `cargo check --workspace --all-targets --locked` |
| `clippy` | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| `test` | `cargo test --workspace --locked` |
| `test-scripts` | Hermetic packaging shell checks and notices tests; macOS freshness/input checks skip on other hosts, notices skip without Python 3.11+ |
| `verify` | Sequential, fail-fast fmt-check → check → clippy → test → test-scripts, even with `make -j` |
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
notices described in [THIRD_PARTY.md](../THIRD_PARTY.md).

`machete` runs `cargo machete` to detect unused direct dependencies. Both targets
are non-mutating with respect to tracked files and fail clearly if the tool is
missing. Cargo and cargo-deny may populate local caches or fetch advisory data;
audit results depend on the current advisory database and registry state.
They are intentionally outside `verify` and the pre-commit hook: these opt-in,
CI-tier dependency checks require extra tools and advisory/network access, while
`verify` remains fmt-check → check → clippy → test → test-scripts.

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

The [macOS artifact recipe](packaging.md#packaging-macos) creates the `.app` bundle; there is no developer
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
See [THIRD_PARTY.md](../THIRD_PARTY.md) for the human-supplied artwork provenance.
