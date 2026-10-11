---
name: changelog
description: Keep CHANGELOG.md current under [Unreleased] as user-facing work lands in Glance, and cut local releases with make bump-version / make release. Use when committing viewer features, format or rendering changes, fixes, packaging/platform changes, when asked to update the changelog, or when preparing a release.
---

# Changelog and release discipline for Glance

`CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
It has one `## [Unreleased]` section at the top and one immutable section per
release. The rule: **a user-facing change and its changelog bullet land in the
same commit.** Do not batch notes for later; later never comes.

## Writing entries

- Add bullets under `## [Unreleased]` using the standard subsections:
  `### Added`, `### Changed`, `### Fixed`, `### Removed`. Create a subsection
  if it does not exist yet; keep subsections in that order.
- One bullet per user-visible change, present tense, backticked identifiers:
  `* Add Space-pan while a zoomed image overflows the viewport`.
- Belongs in the changelog: supported image/document formats and handlers
  (newly supported, or explicitly unsupported); viewer behavior a person
  observes (open/folder browsing, sorting, zoom, Fit/Actual Size, pan,
  animation playback and rate, rotate/mirror/flip, clipboard, reveal); rendering
  fidelity (ICC-to-sRGB normalization, tiled full-detail rendering, dyadic
  minification, retained bit depth, float-TIFF previews); keyboard/mouse input;
  startup and responsiveness changes users feel; platform support (macOS Apple
  Silicon, Linux); packaging and installation (`AppImage`, `make install-linux`,
  DMG/zip, ad-hoc signing, bundled native AVIF, notices); dependency switches a
  user can notice; and security fixes.
- Does not belong: internal refactors with no observable behavior change, test or
  fixture churn, benchmark bookkeeping, formatting, typo-level doc fixes, and CI
  plumbing — except when it changes what users receive (for example, a release
  asset or its integrity). When unsure, one short line under `### Changed` beats
  silence.
- Keep support claims honest: native AVIF is decoder-only; EXR/Radiance HDR,
  PQ/HLG, XCF and AVIF/JXL sequences are unsupported. Do not write a changelog
  entry that claims behavior the shipped build does not have.

## Backfilling entries from history

After a large landed batch, reconstruct entries from the commit subjects and
bodies of the merged work, one bullet per user-visible change, grouped under the
correct subsection. Verify each bullet against the diff (`git show --stat`);
commit subjects are leads, not evidence. Write what an image-viewer user
observes, not how it was built.

## Cutting a release

Never hand-edit version numbers, and never hand-stamp or reorder release
headings. Use the scripts:

```bash
make bump-version BUMP=patch   # or minor|major, or VERSION=x.y.z — bump + stamp only
make release BUMP=patch        # one-shot: bump + commit + annotated vX.Y.Z tag (never pushes)
```

`scripts/bump-version.sh` bumps `[package].version` in `Cargo.toml`, refreshes the
`Cargo.lock` package entry, and inserts `## [x.y.z] - <today>` directly under
`## [Unreleased]`, so the accumulated Unreleased bullets become that release's
notes. It restores `Cargo.toml`, `Cargo.lock` and `CHANGELOG.md` if anything
fails, and an exact `VERSION=x.y.z` retry does not double-stamp an existing
section. `make release` runs that same bump, then commits only those three files
as `chore(release): release vX.Y.Z`, refuses to run over unrelated tracked
changes, creates the annotated tag, rolls back file changes if it fails before
the commit, and never pushes.

With plain `make bump-version`, review the diff, then either commit the three
stamped files and tag `git tag -a vX.Y.Z -m "Glance vX.Y.Z"`, or use
`make release` to do both. The tag must equal `v$(make -s version)`;
`.github/workflows/release.yml` refuses a release whose tag does not match the
package version.

Two things complete a release after tagging:

1. `git push origin main` then `git push origin vX.Y.Z` triggers
   `.github/workflows/release.yml`: **gate → package → publish**. Both gate
   entries must pass `make verify`, `make audit`, `make machete` before either
   packaging job starts, and both packages must succeed before the GitHub
   Release is created. macOS produces arm64 `.zip`/`.dmg` (ad-hoc signed,
   bundled decoder-only AVIF, notices); Linux produces the `.AppImage`, a
   portable `.tar.gz`, and notices archives, all checksummed. Publishing creates
   the release as a draft, uploads, verifies the exact asset set, then publishes.
   There is no crates.io, Developer ID signing, or notarization step.
2. Pushing the branch or the tag is an externally visible mutation. Get explicit
   human authorization before pushing `main` or a release tag; the agent prepares
   and verifies locally (`make verify`, `make bump-version`/`make release` dry
   inspection, artifact smoke) and reports, but does not publish on its own.

## Version semantics

Pre-1.0 (`0.x`): new capabilities or new user-visible surface = **minor**;
fixes and refinements = **patch**; reserve `1.0.0` for the first stable release,
and treat a `0.x` breaking change as a minor bump while documenting it plainly
in the entry. Never rewrite or reorder historical release sections.
