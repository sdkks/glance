.DEFAULT_GOAL := help
.PHONY: help bundle-avif fmt fmt-check check clippy test verify precommit build run icons install-hooks uninstall-hooks version bump-version release

help:
	@printf '%s\n' 'Glance development commands:' '  fmt        Format Rust (modifies files)' '  fmt-check  Check formatting' '  check      Compile all targets without building executables' '  clippy     Lint all targets, warnings are errors' '  test       Run tests' '  verify     Sequential fmt-check, check, clippy, test' '  precommit  Staged whitespace, working-tree fmt-check and check' '  build      Build the release executable' '  bundle-avif Stage macOS decoder-only native AVIF (AVIF_STAGE=new/path)' '  macos-artifacts  Package .app, zip and DMG (macOS tools)' '  dmg        Alias for macos-artifacts' '  macos-smoke  Mount/launch smoke for produced artifacts (IMAGE=path)' '  run        Run the viewer (ARGS="path/to/image.png")' '  audit      Check dependency licenses, advisories, bans, sources (non-mutating)' '  machete    Check for unused direct dependencies (non-mutating)' '  icons      Regenerate macOS and Linux icons from committed masters (macOS tools)' '  install-hooks    Opt in to tracked pre-commit checks (refuses hooksPath conflicts)' '  uninstall-hooks  Remove only our local tracked-hook activation' '  version    Print the package version' '  bump-version  Bump and stamp notes (BUMP=major|minor|patch or VERSION=x.y.z)' '  release    Bump, commit and tag locally (BUMP/VERSION; never pushes)'

install-hooks:
	sh scripts/install-hooks.sh install

uninstall-hooks:
	sh scripts/install-hooks.sh uninstall

icons:
	sh scripts/generate-icons.sh

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

check:
	cargo check --workspace --all-targets --locked

clippy:
	cargo clippy --workspace --all-targets --locked -- -D warnings

test:
	cargo test --workspace --locked

# Recursive recipes run in order and stop on failure, including under make -j.
verify:
	+$(MAKE) fmt-check
	+$(MAKE) check
	+$(MAKE) clippy
	+$(MAKE) test

precommit:
	git diff --cached --check
	+$(MAKE) fmt-check
	+$(MAKE) check

build:
	cargo build --release --locked

run:
	cargo run --locked -- $(ARGS)

# Opt-in audits do not modify tracked files or install developer tools.
.PHONY: audit machete
audit:
	@cargo deny --version >/dev/null 2>&1 || { printf '%s\n' 'cargo-deny missing; see README installation commands.' >&2; exit 1; }
	cargo deny --locked check

machete:
	@cargo machete --version >/dev/null 2>&1 || { printf '%s\n' 'cargo-machete missing; see README installation commands.' >&2; exit 1; }
	cargo machete
# Native AVIF release payload (isolated from development/release targets above).
.PHONY: bundle-avif
AVIF_STAGE ?= build/bundle-libs
bundle-avif:
	bash scripts/bundle-avif-libs.sh "$(AVIF_STAGE)"

# macOS unsigned release artifacts (separate from development/version targets).
.PHONY: macos-artifacts dmg macos-smoke
macos-artifacts:
	sh scripts/package-macos.sh

dmg: macos-artifacts

macos-smoke:
	sh scripts/smoke-macos.sh "$(IMAGE)"
