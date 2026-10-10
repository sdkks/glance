.DEFAULT_GOAL := help
.PHONY: help fmt fmt-check check clippy test verify precommit build run

help:
	@printf '%s\n' 'Glance development commands:' '  fmt        Format Rust (modifies files)' '  fmt-check  Check formatting' '  check      Compile all targets without building executables' '  clippy     Lint all targets, warnings are errors' '  test       Run tests' '  verify     Sequential fmt-check, check, clippy, test' '  precommit  Staged whitespace, working-tree fmt-check and check' '  build      Build the release executable' '  run        Run the viewer (ARGS="path/to/image.png")'

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
