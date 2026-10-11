# Packaging and releases

Build, verification and release instructions for maintainers. For installation and first launch, see [Installing Glance](installing.md). See [Development](development.md) for local quality checks.

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
recorded in [THIRD_PARTY.md](../THIRD_PARTY.md) and verified before extraction. No external
libyuv, AOM, encoder, libvmaf or libheif library is shipped. Upstream's own five
embedded libyuv scaling source files are still compiled with `AVIF_LIBYUV=OFF`;
their BSD-3-Clause notice is in the copied upstream LICENSE. See [THIRD_PARTY.md](../THIRD_PARTY.md)
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
see [Packaging (macOS)](#packaging-macos) for the complete comparison.

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

## Linux source-install verification

For a repeatable, display-free association check from Linux or macOS, run:

```sh
make check-linux-associations
```

This needs **podman and network access** to fetch Ubuntu 24.04 and install
`xdg-utils`, `desktop-file-utils`, and GLib command-line tools. It streams the
committed desktop entry into a disposable container (no host installation or
association changes), validates it, checks PNG/GIF defaults in `mimeapps.list`,
reads them back with `xdg-mime` and `gio mime`, and uses `gio launch` to assert
that `Exec` passes the file path as one argument to a logging shim. It does not
use `xdg-open`, which needs a desktop session. If a built native executable in
`target/{release,debug}` or a native Linux AppImage in `build/` exists, it also
checks real `file://` argument decoding via `GLANCE_SMOKE_IMAGE=1` on the host;
otherwise that sub-check prints SKIP. Missing podman skips the whole check;
assertion failures exit nonzero. This networked check is not part of `make verify`.
Real menu/icon discovery and graphical desktop behavior still need the manual
Linux checks below.

Real Linux menu/icon discovery, associations, native dependencies, and cache
refresh have not yet been verified on the macOS development machine. On Linux, run:

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

Release notes use the tag's `## [X.Y.Z]` body from `CHANGELOG.md` at the tagged commit.
Missing or empty sections fall back to the embedded summary; reruns refresh notes.

Pushing the reviewed branch and `vX.Y.Z` tag starts `.github/workflows/release.yml`:
**gate → package → publish**. Gates run on `ubuntu-26.04` and `macos-26`;
the publish job runs on `ubuntu-26.04`. Both gates must pass
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
where libavif ≥1.4.2 and dav1d ≥1.5.4 are available; older native packages
correctly disable AVIF there. Packaging independently builds checksum-pinned
native decoders, bundles them, and requires the existing AVIF smoke checks.

Release attachments are compressed, never raw executables or `.app` trees:

- macOS (Apple Silicon): `Glance-<version>-arm64.zip`, `.dmg`, and `.sha256`
  (one checksum file covers both zip and DMG); plus
  `Glance-<version>-macos-arm64-notices.tar.gz` and `-notices.sha256`.
- Linux: `Glance-<version>-x86_64.AppImage` and `.AppImage.sha256` (native Ubuntu
  24.04 runner deliberately retained for glibc compatibility; AppImage contains
  compressed SquashFS); plus
  `Glance-<version>-linux-<arch>.tar.gz` (portable AppRun payload and tagged
  source for `make install-linux`), `Glance-<version>-linux-<arch>-notices.tar.gz`,
  and `Glance-<version>-linux-<arch>.sha256` covering both tarballs.
- Notices archives contain the complete notices also included inside the
  application artifacts. Checksum entries use sibling basenames: download all
  attachments into one directory before checking them.

macOS support is **Apple Silicon (arm64)** only: there is no Intel build and no
Universal binary, and the release `package` job refuses to run on a non-arm64
runner. macOS artifacts are
**unsigned by a verified developer** (ad-hoc signed only), so downloaded apps
need the [Gatekeeper steps](installing.md#macos-releases): drag to Applications, then
right-click → Open, or remove only the trusted app's quarantine attribute.
The publish job summary lists each attachment's exact bytes and decimal MB,
total upload size, and PASS/OVER against the <40 MB compressed-DMG target.
Exceeding that target is reported, not hidden by dropping native AVIF.

**Not yet verified until a real tag run:** runner availability of the pinned Rust and
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
`target/macos-artifacts/`: `Glance.app`, `Glance-<version>-arm64.zip`,
`Glance-<version>-arm64.dmg` and `Glance-<version>-arm64.sha256`.
Version comes from `[package].version` in `Cargo.toml`; the macOS shipping
target is Apple Silicon, so `arch` is `arm64` (no Universal binary is produced).
Verify downloads with
`shasum -a 256 -c Glance-<version>-arm64.sha256` in that directory.
The zip uses `ditto --sequesterRsrc --keepParent`; the compressed DMG contains
`Glance.app` and an `/Applications` symlink. Drag the app to Applications.
The recipe is reproducible, but archive bytes/checksums may change with tool
versions or filesystem timestamps; bit-for-bit reproducibility is not claimed.

The bundle identifier is `io.github.glance-viewer.glance` (a stable application
namespace, not a claim of domain ownership). Display name is Glance, executable
is `glance`, and the icon is `Contents/Resources/glance.icns`. Both bundle version
keys use the Cargo version. The declared and compiled minimum macOS version is
**11.0**, the first Apple Silicon macOS;
actual oldest-OS compatibility remains unverified. Imported image UTIs cover
all eligible extensions in `src/formats.rs` (including flattened documents),
with role **Viewer** and handler rank **Alternate**, never Owner/Default.
Registration does not change existing default associations. Glance handles native
Finder document-open events: double-clicking an associated image or dropping
files onto its Dock icon opens the first local document, whether Glance is
already running or launched by the event. CLI image arguments retain precedence
at startup. Real Finder/Dock delivery and reply timing, including on macOS 11,
remain unverified; Open/drop inside the window and CLI opening remain available.

Native AVIF is bundled **by default**: `make macos-artifacts` runs `make
bundle-avif` when `AVIF_STAGE` (default `build/bundle-libs`) does not exist,
then `make notices` using that stage's native notices. An existing stage is reused
only if its executable is not older than the Rust sources, Cargo manifests/lockfile,
`build.rs`, and embedded macOS icon. Newer inputs automatically refresh a stage
under the repository's `build/` directory: the old stage is removed and rebuilt,
with a notice in the build log. Automatic removal requires a relative child path
of `build/`, no symlink or traversal components, and the
`Contents/MacOS/glance` payload marker. Unsafe or incomplete paths are refused
without deletion; remove/refresh them manually. Up-to-date stages are left alone.
The builder itself still refuses to reuse a directory. For unusual situations
(such as a stale stage outside `build/`), build into a new, empty path:

```sh
make bundle-avif AVIF_STAGE=build/avif-release-2
make macos-artifacts AVIF_STAGE=build/avif-release-2
```

Choose a fresh `AVIF_STAGE` after toolchain changes too (timestamps do not detect
those). Partial/missing stage inputs fail loudly, never silently
produce an AVIF-less release. Direct `sh scripts/package-macos.sh [stage]`
consumes an already prepared stage and notices; it does not prepare them and
still refuses stale executables with manual recovery commands.

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
Actual runtime behaviour on macOS 11–14 (Apple Silicon) has not yet been verified;
loader metadata and current arm64 smoke are not older-OS runtime testing.

These builds are **ad-hoc signed, but not Developer ID signed or notarized**.
Packaging signs the completed bundle before archiving and verifies its signature;
`ditto` preserves that signature in the zip. Missing `codesign` or signing failure
stops packaging unless you explicitly set `ALLOW_UNSIGNED=1`, which prints a
warning and permits artifacts without a verified signature. The smoke check still
requires valid signatures.

For [installation and Gatekeeper steps](installing.md#macos-releases), see the installation guide.

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
make test-scripts
```

`make test-scripts` runs the portable macOS deployment-floor parser checks,
macOS-only freshness and packaging-input checks, then the notices unit tests.
The macOS-only checks print a skip note on other hosts; notices tests print a
skip note when Python 3.11+ (`tomllib`) is unavailable. These hermetic checks
need no native build, network access, or real system mutation and also run as
the final step of `make verify`.

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
whole Linux distribution. The Linux AppImage deliberately builds on the
`ubuntu-24.04` baseline for glibc compatibility, not arbitrary older Linux;
`ubuntu-26.04` would raise the glibc floor from 2.39 to 2.43. CI/release gates
use `ubuntu-26.04` and `macos-26`, and publication uses `ubuntu-26.04`. Normal AppImage mounting needs working FUSE (`/dev/fuse`). Older runtimes
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
and desktop/icon layout. **Not yet verified**: x86_64 runtime/build, real Ubuntu desktop
launch, file-manager Open With, folder-wide next/previous and GPU/driver behavior.
On a stock Ubuntu desktop run `./Glance-<version>-<arch>.AppImage <file>`, open a
folder, use Left/Right and Home/End, and open an image via the file manager;
confirm the icon, title/count, neighboring files and AVIF rendering.

EXR/Radiance HDR, PQ/HLG, XCF, AVIF/JXL sequences and Windows remain unsupported;
packaging does not enable those formats or HDR output.

## Licensing

Glance is MIT; see [LICENSE](../LICENSE). Direct dependencies are pinned to eframe 0.36.2
(MIT OR Apache-2.0), image 0.25.10 (MIT OR Apache-2.0), rfd 0.17.2 (MIT), arboard
3.6.1 (MIT OR Apache-2.0), and the macOS-only objc2 0.6.5 (MIT), objc2-app-kit 0.3.2
(Zlib OR Apache-2.0 OR MIT), and objc2-foundation 0.3.2 (MIT).
The viewer directly enables image's JPEG/PNG/TIFF/BMP/ICO/TGA/PNM/GIF/WebP codecs
with defaults disabled, plus image-extras 0.1.1 (MIT OR Apache-2.0) with the selective
features listed in [Raster and icon formats](formats.md#raster-and-icon-formats). Color conversion uses
moxcms 0.8.1 (BSD-3-Clause OR Apache-2.0); RGBA16F packing uses half 2.7.1
(MIT OR Apache-2.0), promoted from the existing resolved graph. Settings parsing
uses toml_edit 0.25.17 (MIT OR Apache-2.0), also promoted without new resolved
packages. egui's bundled
default fonts have their own notices.
See [THIRD_PARTY.md](../THIRD_PARTY.md) for the resolved crate/license inventory and distribution
limitations. No third-party library source has been modified.
