# Contributing to Glance

Thanks for helping improve Glance. This guide is for contributors working on
code, tests or documentation.

## Start here

Read [Installing Glance](docs/installing.md) for build prerequisites and running
from source. Then read [Development](docs/development.md) for local checks,
dependency audits and the tracked hook.

From your checkout's root, install the pinned Rust **1.97.1** toolchain with
rustfmt and Clippy, plus the platform libraries described in the installation
guide. macOS needs Apple command-line tools. Linux needs windowing and graphics
development libraries and a working X11 or Wayland desktop.
Native AVIF needs pkg-config, libavif ≥1.4.2 and dav1d ≥1.5.4; without them,
the build succeeds but AVIF is unavailable.

Install the audit tools using the tested versions in
[Development](docs/development.md#opt-in-dependency-audits). Run `make help`
to see the available commands.

## Run from source

```sh
make run ARGS='/absolute/path/to/image.png'
# Or:
cargo run --locked -- /absolute/path/to/folder
make build
./target/release/glance /absolute/path/to/image.png
```

## Check before pushing

Install the tracked **pre-commit** hook locally:

```sh
make install-hooks
```

The hook runs `make precommit`: staged whitespace checks, `fmt-check` and `check`.
It runs before a commit, not automatically on push. Install it before committing
and pushing your changes. If another hooks path is active, installation refuses
to replace it; see the [hook setup and caveats](docs/development.md#opt-in-pre-commit-hook).

Before pushing and opening a pull request, run the full checks:

```sh
make verify
make audit
make machete
```

`make verify` runs formatting checks, compilation, Clippy, tests and script tests
in order. The hook is lighter than this full verification. With partial staging,
formatting and compilation check the whole working tree, not an isolated staged
snapshot. Audits need extra tools and may need network access; they are not part
of `make verify` or the hook.

## Open a pull request

- Keep changes small and focused. Explain what changes and why.
- Add tests for behaviour changes, and include the checks you ran.
- For each user-visible change, add one bullet under `[Unreleased]` in
  [CHANGELOG.md](CHANGELOG.md), in the same commit as the change. Follow the
  [changelog guidance](.pi/skills/changelog/SKILL.md#writing-entries).
- Avoid unrelated generated-file churn. Do not commit build outputs.
- Update the relevant documentation when behaviour changes.

**Releases are cut only by the repository maintainer.** Contributors do not bump
versions, create release tags, or push releases. The maintainer's runbook is
[Releasing Glance](docs/releasing.md).

## Report a bug

Open an issue at https://github.com/sdkks/glance/issues. Include:

- The version and commit SHA shown in **Help → About Glance**.
- Your operating system and how you installed or built Glance.
- Steps to reproduce the problem, what you expected and what happened.
- Relevant error messages and, when possible, an image that reproduces the issue.

## Documentation map

- [README](README.md) — quickstart and project overview.
- [Installing Glance](docs/installing.md) — downloads, first launch and source builds.
- [Using Glance](docs/usage.md) — controls, playback, settings and desktop actions.
- [Supported formats and color](docs/formats.md) — formats, depth and codec caveats.
- [Rendering and limits](docs/rendering.md) — full detail, minification and resource bounds.
- [Development](docs/development.md) — checks, audits, hooks, icons and install verification.
- [Packaging](docs/packaging.md) — artifact construction and distribution notices.
- [Releasing Glance](docs/releasing.md) — maintainer-only versioning and release steps.
