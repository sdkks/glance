.DEFAULT_GOAL := help
.PHONY: help version bump-version release bundle-avif fmt fmt-check check clippy test test-scripts verify precommit build run icons install-hooks uninstall-hooks version bump-version release

help: help-linux
	@printf '%s\n' 'Glance development commands:' '  fmt        Format Rust (modifies files)' '  fmt-check  Check formatting' '  check      Compile all targets without building executables' '  clippy     Lint all targets, warnings are errors' '  test       Run tests' '  test-scripts Run hermetic packaging and notices tests (macOS checks skip elsewhere)' '  verify     Sequential fmt-check, check, clippy, test, test-scripts' '  precommit  Staged whitespace, working-tree fmt-check and check' '  build      Build the release executable' '  bundle-avif Stage macOS decoder-only native AVIF (AVIF_STAGE=new/path)' '  macos-artifacts  Package .app, zip and DMG (macOS tools)' '  dmg        Alias for macos-artifacts' '  macos-smoke  Mount/launch smoke for produced artifacts (IMAGE=path)' '  run        Run the viewer (ARGS="path/to/image.png")' '  audit      Check dependency licenses, advisories, bans, sources (non-mutating)' '  machete    Check for unused direct dependencies (non-mutating)' '  icons      Regenerate macOS and Linux icons from committed masters (macOS tools)' '  install-hooks    Opt in to tracked pre-commit checks (refuses hooksPath conflicts)' '  uninstall-hooks  Remove only our local tracked-hook activation' '  version    Print the package version' '  bump-version  Bump and stamp notes (BUMP=major|minor|patch or VERSION=x.y.z)' '  release    Bump, commit and tag locally (BUMP/VERSION; never pushes)' '  appimage   Build the Linux AppImage (native Linux host; see README)'

version:
	@awk '/^[ \t]*\[/ {package=($$0 ~ /^[ \t]*\[package\][ \t]*(#.*)?$$/)} package && /^[ \t]*version[ \t]*=/ {split($$0,v,"\""); print v[2]}' Cargo.toml

bump-version:
	@sh scripts/bump-version.sh "$(BUMP)" "$(VERSION)"

release:
	@sh scripts/release.sh "$(BUMP)" "$(VERSION)"

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

test-scripts:
	sh scripts/tests/test-release-notes.sh
	sh scripts/tests/test-macos-floor.sh
	sh scripts/tests/test-macos-freshness.sh
	sh scripts/tests/test-macos-inputs.sh
	sh scripts/tests/test-macos-dmg-extras.sh
	sh scripts/tests/test-avif-stage-refresh.sh
	@if python3 -c 'import tomllib' >/dev/null 2>&1; then \
		python3 -m unittest discover -s scripts/tests -p 'test_notices.py'; \
	else \
		printf '%s\n' 'SKIP: notices tests require Python 3.11+ (tomllib unavailable).'; \
	fi

# Recursive recipes run in order and stop on failure, including under make -j.
verify:
	+$(MAKE) fmt-check
	+$(MAKE) check
	+$(MAKE) clippy
	+$(MAKE) test
	+$(MAKE) test-scripts

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
# Reproducible distribution notices (Python 3.11+ is build-time only).
.PHONY: notices
notices:
	python3 scripts/generate-notices.py $(NOTICES_ARGS)
# Linux source-prefix installation (independent of artifact packaging).
PREFIX ?= $(HOME)/.local
DESTDIR ?=
DRY_RUN ?= 0
SUDO ?= 0
.PHONY: install-linux uninstall-linux help-linux check-linux-associations
help-linux:
	@printf '%s\n' '  install-linux    Build/install into PREFIX (default ~/.local; DRY_RUN=1 previews)' '  uninstall-linux  Remove unchanged manifest-owned files from PREFIX' '  check-linux-associations  Check MIME defaults and desktop dispatch (podman/network)'

check-linux-associations:
	@sh scripts/check-linux-associations.sh

install-linux:
	@sh scripts/install-linux.sh install "$(PREFIX)" "$(DESTDIR)" "$(DRY_RUN)" "$(SUDO)"

uninstall-linux:
	@sh scripts/install-linux.sh uninstall "$(PREFIX)" "$(DESTDIR)" "$(DRY_RUN)" "$(SUDO)"
# macOS artifact inputs: ordered preparation, separate from existing targets.
.PHONY: macos-artifact-inputs
NOTICES_STAGE ?= build/notices
export AVIF_STAGE NOTICES_STAGE SKIP_AVIF SKIP_NOTICES MACOS_OUT
macos-artifacts: macos-artifact-inputs
macos-artifact-inputs:
	@test "$$(uname -s)" = Darwin || { echo "macOS packaging requires macOS." >&2; exit 1; }
	@if [ "$(SKIP_AVIF)" != 1 ]; then sh scripts/prepare-avif-stage.sh "$(AVIF_STAGE)" $(MAKE); fi
	@if [ "$(SKIP_NOTICES)" != 1 ]; then \
		if [ "$(SKIP_AVIF)" = 1 ]; then native=build/no-native-notices; else native="$(AVIF_STAGE)/Contents/Resources/licenses/native"; fi; \
		$(MAKE) notices NOTICES_ARGS="--output \"$(NOTICES_STAGE)\" --native-notices \"$$native\""; \
	fi
# Linux AppImage packaging (native Linux host; see README).
.PHONY: appimage
appimage:
	sh scripts/package-appimage.sh
