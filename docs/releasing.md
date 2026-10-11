# Releasing Glance

Releases are cut only by the repository maintainer. Contributors do not bump
versions, create release tags, or push releases. See [Contributing](../CONTRIBUTING.md)
for pull-request guidance and [Packaging](packaging.md) for artifact construction.

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
Follow the [macOS packaging smoke recipe](packaging.md#packaging-macos) and
[Linux verification recipe](development.md#linux-verification-recipe) on downloaded assets.
For a negative check on the test repository, create another unused release
version on a scratch branch, deliberately add a failing Rust test before
cutting its tag, and push that scratch branch/tag: the gate must fail, package
and publish must be skipped, and no release/assets must be created for that tag.
Do not move an existing production tag to perform these checks.
