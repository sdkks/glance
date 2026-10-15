# Historical gutter profiling baseline

This is a **nonshipping reconstruction**, not a profiler for current Glance. The
complete overlay applies only to commit
`a975449e4dd138ea4be84946b443f664a5d6cf59` (`v0.4.1-8-ga975449`: v0.4.1 plus
eight commits), **not v0.4.1 itself, v0.4.2, or current main**. It preserves the
original scalar/row-span gutter comparison and single-window diagnostic hooks.
Nothing here is imported by the normal application or enabled in its build.

## Contents and provenance

- `overlay.patch`: the complete historical tracked-source diff **plus** the
  originally untracked `src/profile.rs` and `src/tiles/comparison.rs`.
- `source.sha256`: relative SHA-256 hashes of the ten reconstructed Rust files,
  `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, and `build.rs`.
- `.gitattributes`: preserves the patch's literal bytes and exempts only its
  trailing whitespace from Git's whitespace check (blank diff context lines
  necessarily contain a space). It does not change application-source checks.
- `run-synthetic.sh`: a Bash runner using Cargo's test interface, not a cached
  binary name. It verifies the base and source, compiles, then runs exactly the
  ignored synthetic release benchmark with one test thread.

The patch and manifest are retained byte-for-byte; instrumentation, oracle,
fixtures, timing boundaries and hashes have not been modernized. All test bytes
are generated synthetically by the preserved Rust module. No private images,
GUI logs, binaries, machine-specific launch recipes or copied current-main
fixtures are included. Old timing reports are not new measurements; this package
makes no historical speedup claim and does not bundle a machine-specific timing
exemplar. Generate fresh logs for the machine/configuration being investigated.

Package checksums:

```text
86eec86053aa6e07f74c2b89f59dfe01bafa48c8547dfa759bae82c5f9394a4c  overlay.patch
8cf2e43d3939f67d269c6034e81a63f8dae585e72333e35c17b51346b587d6c3  source.sha256
```

## Prerequisites and isolation

Use macOS or Linux with Bash 3.2+, Git, `sha256sum` or `shasum`, and the pinned
Rust **1.97.1** toolchain. This still builds the historical desktop crate even
though the checks below open no window: macOS needs Apple command-line tools;
Linux needs its windowing/graphics development libraries. See the reconstructed
checkout's `docs/installing.md` and `docs/packaging.md` for native prerequisites.
Native AVIF is optional; the synthetic gutter test does not decode AVIF. Cargo
may fetch the pinned dependencies if they are not cached. No new dependency is
introduced by this package.

Use a separate detached checkout. Do not apply the patch to your main checkout,
installed app, or a different revision. Keep the original package outside the
reconstruction; do not edit the reconstructed source while verifying/timing.
Use the checkout's own ordinary `target/` directory, not a symlink or another
checkout's target cache. Use a normal native Cargo configuration (no cross-target
or custom runner), and record any deliberate compiler configuration separately.
Do not build or run competing benchmarks during timing. These are source checks,
not a hermetic build or a guarantee of identical timings across hosts.

## Reconstruct explicitly

The following is a **manual, opt-in** example, starting at the repository root
that contains this package. Choose a new checkout path; Git refuses an existing
one. The script itself never creates/removes worktrees, applies patches, resets,
cleans, stages, commits, or installs anything.

```bash
repo=$(pwd -P)
package="$repo/tools/profiling/gutter-baseline"
base=a975449e4dd138ea4be84946b443f664a5d6cf59
replay="$repo/.worktrees/gutter-baseline-a975449"
git worktree add --detach "$replay" "$base"
cd "$replay"
test "$(git rev-parse HEAD)" = "$base"
test -z "$(git status --porcelain)"
git apply --check "$package/overlay.patch"
git apply "$package/overlay.patch"
if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "$package/source.sha256"
else
    shasum -a 256 -c "$package/source.sha256"
fi
```

Run commands in order and stop on any failure. All 14 hash entries must pass.
The resulting dirty checkout is intentional: eight modified tracked Rust files
and two new Rust modules. Manifests, lockfile, toolchain and build script remain
at the pinned base. Do not regenerate the patch against newer code or update a
hash to hide a mismatch.

## Headless checks and synthetic timing

In the same shell, create one fresh log parent. All commands preserve complete
stdout/stderr. Compilation is separate from operation timing; run these serially.

```bash
logs=$(mktemp -d "${TMPDIR:-/tmp}/glance-gutter.XXXXXX")
printf 'Logs: %s\n' "$logs"
cd "$replay"
unset GLANCE_PROFILE_SECONDS GLANCE_PROFILE_DETAILS GLANCE_PROFILE_TIMERS GLANCE_PROFILE_TEST_MODE GLANCE_GUTTER_KERNEL
cargo test --release --locked --bin glance --target-dir "$replay/target" --no-run > "$logs/build.log" 2>&1
cargo test --release --locked --bin glance --target-dir "$replay/target" tiles:: -- --test-threads=1 > "$logs/tiles.log" 2>&1
cargo test --release --locked --bin glance --target-dir "$replay/target" profile::tests:: -- --test-threads=1 > "$logs/profile.log" 2>&1
GLANCE_GUTTER_KERNEL=scalar cargo test --release --locked --bin glance --target-dir "$replay/target" render::tests:: -- --test-threads=1 > "$logs/render-scalar.log" 2>&1
GLANCE_GUTTER_KERNEL=row-span cargo test --release --locked --bin glance --target-dir "$replay/target" render::tests:: -- --test-threads=1 > "$logs/render-row-span.log" 2>&1
bash "$package/run-synthetic.sh" "$replay" "$logs/run-1"
```

Stop on a nonzero exit and inspect the named log. The tile checks compare both
kernels byte-for-byte against the frozen scalar oracle at both byte depths, with
small tiles and production boundary/heavy geometry cases. They also check
initialized allocation and capacity overflow without inducing real OOM. Profiler
checks cover bounded collection, percentiles, controls and auto-stop without a
window. Renderer checks cover CPU-side batching and gutter payload handling;
they do **not** exercise a real GPU, presentation, or visible output. No ordinary
full application test suite or other ignored benchmark is run by the runner.

The runner requires two arguments: the detached reconstructed root and a
**nonexistent** output directory whose parent already exists. It rejects a wrong
base, altered hashed source, other tracked edits, unexpected nonignored untracked
files, or an existing output destination (including symlinks). It uses the
checkout's own `target/`, with no cleanup. Verification failures after output
creation leave their logs there; retry with a different output name.

Output includes `verification.log`, `context.log` (UTC time, OS/architecture,
Rust/Cargo versions, base and package hashes), `build.log`, and `benchmark.log`.
The script clears the historical profiling controls for the benchmark; it never
opens a GUI or accepts an image argument. The benchmark itself explicitly tests
both kernels. A fresh run normally spends at least 32 seconds in timing blocks,
plus compilation and setup. Exit status reports verification/build/test success,
**not** a performance threshold. Keep logs private until reviewed: Cargo/errors
can contain local paths even though fixtures and benchmark rows are synthetic.

## What the numbers mean

The ignored `tiles::comparison::benchmark_gutters` test generates a 1280×2052
level, tests 4- and 8-byte pixel layouts and full, partial and corner tiles. For
each case/mode/kernel it performs ten warmups and five approximately 200 ms wall
blocks, alternating scalar/row-span order. Modes are `allocation-fill` and
`fill-only`. `GUTTER` rows include geometry, mode, block/order, kernel, iteration
count, `measured_ns`, `block_wall_ns`, and per-iteration `mean_ns`.

- Operation timing includes allocation/zeroing only in `allocation-fill`.
  Consumption/black-box observation and destruction of returned buffers are
  **outside** `measured_ns`; block wall time includes them and loop overhead.
  Fill-only reuses a buffer. Neither mode is complete application frame cost.
- These are CPU elapsed/wall measurements, not CPU utilization, GPU execution,
  scanout, end-to-end input latency, or achieved screen FPS. Clock/instrumentation
  overhead, allocation state, caches, scheduling, thermals and system load matter.
- The diagnostic GUI reports CPU admission events, not presented frames.
  `admissions_per_s` is over the first-to-last admission span, not the whole
  process lifetime and not proof of display throughput or an FPS guarantee.
- Profiler leaves are nested inside upload/batch/callback envelopes. Do not sum
  parents and children, independent percentile summaries, or overlapping event
  intervals as if they were disjoint. Residuals include bookkeeping, destruction
  and clock overhead; they are not another GPU stage. Failed batches can include
  partial work/drain; attribution overlap is reported separately.
- Each profiler stage/phase retains at most 32,768 samples. Inspect `n`, `dropped`
  and `retained_sum_ms`; retained sums exclude dropped samples. Timer phase is
  selected at envelope start, and startup/end boundaries can leave unmatched
  request/ready/taken events. Inspect pending-event flags, geometry/playback
  changes and auto-stop before comparing runs. Stage counts need not align.
- Means, medians, nearest-rank p95 and worst retained samples describe only the
  retained population. They do not establish a general speedup or performance
  assertion gate. Compare repeated like-for-like runs, not isolated best cases.

## Separately invoked diagnostic GUI (not part of the runner)

The complete overlay retains GUI hooks for historical source reconstruction;
the default workflow above is headless only. A deliberately launched reconstructed
app enables them only when `GLANCE_PROFILE_SECONDS` is set to an integer 1–120.
That mode uses **session-only default settings**, not saved preferences (default
Fit and honor-file playback), and requests automatic application close when the
interval expires. It reports to stderr on application exit. The interval starts
at profiler initialization, so startup is included; auto-close depends on the
application event loop, not an external kill timer. Keep it to one diagnostic
window and original synthetic inputs; never treat raw GUI logs as public data.

`GLANCE_GUTTER_KERNEL` chooses `scalar` (default) or `row-span`.
`GLANCE_PROFILE_DETAILS=0` suppresses leaf attribution while retaining outer
timers; `GLANCE_PROFILE_TIMERS=0` selects events-only observation. Otherwise,
profiling enables detailed timers. Invalid kernel values panic, as does an
invalid duration. With the duration unset the diagnostic profiler is disabled
and the reconstructed GUI follows its ordinary settings path; it is **not** a
session-isolated launch. Do not install this historical binary or confuse it
with current-main behavior. GUI/presentation validation is not claimed here.
