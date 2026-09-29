---
title: "Testing and CI: harnesses, fuzz targets, path-filtered CI and a bench script"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M1"
resolved: "2026-09-29"
---

# Testing and CI

Filed 2026-09-29. Serves **daily-drive** (a bar people depend on) and the
engineering bar in `CLAUDE.md`: tests land with each piece, not batched.
Modeled on scootbg (`crates/scootbg/tests`, `crates/scootbg/fuzz`,
`scripts/scootbg-bench`, the `scootbg` path filter in `.github/workflows/ci.yml`).

## Layers

- **Pure drawing** (no Wayland): snapshot tests of canvas output for rects,
  rounded rects, glyph runs at 1x and a fractional scale.
- **Module harness**: fake events in, assert on the `View` and the returned
  `Action`; every module ships tests through it.
- **Integration** against real compositors: headless scoot (screenshot over
  IPC, pixel assertions, hotplug with `--outputs N`) and headless sway, as
  scootbg's suite does. Tests skip without a compositor binary unless
  `SCOOTBAR_REQUIRE_SCOOT=1` makes that a failure.
- **Fuzz targets, each landing with the parser it covers**: the clock format string
  and the TZif reader (M1); the config parser (M3); the `exec`/`push` JSON (M4);
  the D-Bus message parser if hand-rolled (M6). Regression inputs kept in-tree, as
  scootbg's `regressions/`.

## CI

A `scootbar` filter in the workflow's classify job so a bar-only PR skips
unrelated jobs; `cargo nextest run` and `cargo test` for the package
(`CLAUDE.md` explains why both); clippy `-D warnings` and `fmt --check`; the
Cargo-feature matrix (default, `--no-default-features`, each module alone) so
the smallest build stays buildable; an `ldd` assertion that the default build
links only libc, libm and libgcc_s (no libEGL or libgbm); the macOS `cargo
check` of the stub.

## Benchmark

`scripts/scootbar-bench`, reusing `scripts/scootbg-bench`'s Python runner
(procs, sizing, report), for the [resource ratchet](../lightest.md).

## Done when

A bar-only PR runs exactly the jobs it needs, every module has harness tests,
and every fuzz target that exists runs for a fixed budget without findings.

## Resolution

Resolved 2026-09-29 on branch `feat/testing-and-ci`. Much of the ticket had
landed with the entries before it (the integration suites on headless scoot
and sway with `SCOOTBAR_REQUIRE_*`, the `scootbar` path filter, both test
runners, clippy, fmt, `--no-default-features`, the `libc`-crate and `ldd`
checks, the fuzz targets and their corpus); this entry adds what was
missing and makes the rest hold for modules not written yet. How to run all
of it: [testing.md](../../testing.md).

### What landed

- **Snapshots** (`crates/scootbar/src/snapshots.rs`, scenes in
  `snapshots/tests.rs`, images beside them): whole canvases compared pixel
  for pixel with checked-in ASCII PGM/PPM, one image row per line, so a
  change is a readable diff. `fills.ppm`: span fills (overlapping, empty,
  clipped at the edge and at the end of `u32`) and coverage blends inside
  and outside a clip. `bar-1x.pgm`, `bar-1.5x.pgm`,
  `bar-clipped-1.25x.pgm`: a three-section bar through the whole render
  path in the seven-segment test font (an icon, `.notdef`, three classes)
  at 1, 1.5, and 1.25 too narrow so its sections clip. `SCOOTBAR_BLESS=1`
  rewrites them; without it a missing image fails, so CI cannot bless one;
  a failure writes what was drawn and prints both as text; an image no
  test compares fails.
- **The module harness**: `Harness::deliver(source, events)` hands any
  events to any source the module added, with no poll (a wake with
  nothing to read, `POLLERR`, `POLLHUP`, `POLLNVAL`), and
  `Harness::start` starts a registry entry as the bar does. Two tests in
  `modules/tests.rs` hold **every module in the registry** with no new
  code: it survives every event the loop can hand it on each source and
  still fills a bounded view (or, unavailable, says why), and it has a
  `modules/<id>/tests.rs` that drives it through the harness. The clock's
  spurious-wake test goes through the harness now, and a new one shows an
  error or hang-up on its timer leaves it ticking (the next real tick
  arrives).
- **Fuzzing**: what each target checks is written once
  (`src/modules/clock/fuzz.rs`), compiled by the fuzz crate by `#[path]`
  (each target is now one line) and by a stable test that replays the seed
  corpus and every file in `fuzz/regressions/` on every `cargo test`
  (nothing read `regressions/` before).
- **CI** (`.github/workflows/ci.yml`): the `scootbar` job clippies each
  module alone besides the default and no-module builds, with the modules
  read from `Cargo.toml` by `cargo metadata | jq`, so a new module joins
  the matrix with no workflow change; runs the benchmark harness's unit
  tests; and builds and runs both fuzz targets for a fixed budget
  (1,000,000 runs of `format`, 5,000,000 of `tzif`, `-seed=1`, over the
  corpus and `regressions/`), printing a finding's input in base64.
  `scootbar-macos` checks the macOS stub (`--all-targets`, with and
  without modules) when the bar changed and the compositor did not; the
  `macos` job's workspace check covers every other case, including a
  failed classification. The `scootbg` job gains `cargo check` of its fuzz
  crate. `scripts/scootbar-bench/` classifies as `scootbar`.
- **The benchmark**, `scripts/scootbar-bench/` (`bench.py run | report |
  compare`): scootbar against yambar and Waybar from the pinned nixpkgs,
  each showing exactly the milestone's scope (the clock, in M0's look), on
  headless scoot and sway with two `foot` windows on two workspaces:
  startup to first frame, idle RSS, PSS, heap, peak, wakeups and CPU, CPU
  while switching workspaces, and Size (binary plus non-glibc closure).
  `report` is the ratchet's rule 2 (no competitor ahead), `compare` its
  rule 1 (no regression against an earlier run). It imports
  `scripts/scootbg-bench`'s `session`, `commits`, `procs`, `size`,
  `report` (the noise rule) and `daemons.nix_resolve`; the bars, stage,
  rows and tables are its own (`bars.py`, `stage.py`, `measure.py`,
  `tables.py`), with unit tests of that logic. M1's run is checked in as
  the baseline, `docs/scootbar/bench/m1-clock/`, as M0 kept its raw
  results.
- **Docs**: [testing.md](../../testing.md) (snapshots, the harness, fuzzing
  in CI, the CI jobs, the benchmark), both fuzz READMEs,
  [appearance](../appearance.md) (the rounded-rectangle snapshots),
  [the M1 follow-ups](../m1-review-followups.md) (item 3 done), and the
  README's status line and baselines.

### From the M1 review follow-ups

Pulled in, being small and inside this ticket: **item 3**, the fuzz crates
never compiled in CI. scootbar's two targets are now built and run in the
`scootbar` job, and scootbg's crate is checked in the `scootbg` job.

Left, with why: **item 4**, a counting-allocator test of "a warm tick
allocates nothing": scootbar is `#![forbid(unsafe_code)]`, a
`GlobalAlloc` needs an `unsafe impl`, and an integration test cannot reach
a binary crate's internals, so it needs a decision of its own (where the
allocator lives), not a line here. Items 1, 2 and 5 are not testing
infrastructure.

### Deviations

- **No rounded rectangles in the snapshots**: the canvas has none yet (it
  fills spans and blends coverage). They land with the primitive, in
  [appearance](../appearance.md), which now says so. Adding a primitive
  only to snapshot it would be speculative.
- **"The returned `Action`"** is the module API's `Update`
  (`Changed`/`Unchanged`); nothing was renamed.
- **"Each module alone"** is one build today (the clock alone is the
  default build); the loop is what scales.
- **The fuzz budget is a number of runs, not a time**: a fixed seed and run
  count make a CI run the same run every time, where a time budget would
  make a finding depend on the runner's speed.
- **The macOS job could not be run here**: this container has no Darwin
  standard library. The same `cfg(not(target_os = "linux"))` path was
  checked for `wasm32-unknown-unknown` (below); the real check is CI's.
- **The benchmark runs yambar and Waybar, not ironbar and ashell**: the
  ratchet names those two for this scope, and M0 ran the others as "if
  cheap". A class each in `bars.py` adds them.
- **Startup times from this script are not M0's**: it times the bar's
  first frame where the compositor received it, with the compositor
  printing its whole protocol (`WAYLAND_DEBUG=server`, scootbg's method),
  which slows every bar's round trips alike; M0 read the bar's own debug
  output. The other rows reproduce M0's numbers (below). The README keeps
  M0's tables and adds this run's.

### Evidence

A Claude Code web container (4 vCPU, Intel Xeon @ 2.80 GHz, Linux
6.18.44), worktree `/tmp/impl-testci`, `CARGO_TARGET_DIR=/tmp/impl-testci-target`,
every cargo command through `devenv shell --`. Neither dev VM answered
(`nc -z localhost 31022`, `nc -z localhost 2222`); nothing here needs
`--tty`. **The code is final at `3801c12`** (and unchanged from `5cbc4bf`,
whose successor only touched docs); every later commit is docs only.

**The workspace**, at `5cbc4bf`, with sway given and both compositors
required:

```
$ SCOOTBAR_TEST_SWAY=.../sway-1.12/bin/sway SCOOTBG_TEST_SWAY=... SCOOTBAR_REQUIRE_SCOOT=1 SCOOTBAR_REQUIRE_SWAY=1 \
    devenv shell -- soft-egl cargo nextest run --workspace --no-fail-fast
    Starting 2841 tests across 26 binaries (28 tests skipped)
     Summary [ 209.988s] 2841 tests run: 2841 passed, 28 skipped
real 7m58.155s; rc=0
```

**scootbar**, at `3801c12` plus the docs-only changes after it, the same
environment:

```
$ cargo nextest run -p scootbar --no-fail-fast
    Starting 178 tests across 4 binaries
     Summary [  11.659s] 178 tests run: 178 passed, 0 skipped
$ cargo test -p scootbar
test result: ok. 156 passed; 0 failed   (unit)
test result: ok. 9 passed; 0 failed     (tests/bar.rs)
test result: ok. 8 passed; 0 failed     (tests/clock.rs)
test result: ok. 5 passed; 0 failed     (tests/hotplug.rs)
$ cargo clippy -p scootbar --all-targets -- -D warnings                              rc=0
$ cargo clippy -p scootbar --no-default-features --all-targets -- -D warnings        rc=0
$ cargo clippy -p scootbar --no-default-features --features clock --all-targets -- -D warnings   rc=0
    (the feature list as CI reads it: cargo metadata | jq ... -> "clock")
$ cargo clippy -p scootbg -p scootbg-mem --all-targets -- -D warnings                rc=0
$ cargo clippy -p scoot --all-targets -- -D warnings                                 rc=0
$ cargo fmt --check -p scootbar / -p scootbg -p scootbg-mem / -p scoot               rc=0 each
$ cargo fmt --check --manifest-path crates/scootbar/fuzz/Cargo.toml                  rc=0
$ cargo check --manifest-path crates/scootbg/fuzz/Cargo.toml --target-dir ... --locked   rc=0 (19.7 s cold)
$ python3 -m unittest discover -s scripts/scootbar-bench -p 'test_*.py'             Ran 10 tests, OK
$ python3 scripts/test_backlog.py                                                    Ran 30 tests, OK
```

`scripts/backlog check` reports three problems, the same three on
`origin/main`'s own tree (two `protocol-gaps-*` entries resolved but not
archived, and `multi-output-foundation-done.md` missing `blocked`), none
from this change. `scripts/smoke-test.sh` was not run: it drives the
compositor and scootbg, and nothing it runs changed.

**The release bar is unchanged.** Built from the same directory, each in
its own target dir so cargo could not reuse the other's build (a first
attempt sharing one was reused, as `CLAUDE.md` warns, and is discarded):

```
origin/main b3f087b 3beff4ebabd48cf18fb994d0c068bdceac6d7b092914148bb5c63b8521be2b49 848624
HEAD        3801c12 3beff4ebabd48cf18fb994d0c068bdceac6d7b092914148bb5c63b8521be2b49 848624
```

Built from different directories the hashes differ in 32 bytes at the same
size, most likely through cargo's metadata hash for a path crate, which
includes its location; the same-directory builds above rule out the code.

**The workflow**: `actionlint` 1.7.12 with shellcheck 0.11.0 (both from the
pinned nixpkgs) reports the same two SC2174 warnings on `main`'s file and
this one, both in the Linux jobs' `mkdir -p -m`, and nothing new (a planted
error in a scratch workflow was reported, so it runs). It does not
shellcheck the `nix develop` steps, so the new clippy and fuzz scripts
were extracted and run through shellcheck directly: clean. The fuzz step's
script, run as CI runs it (`GITHUB_WORKSPACE` and `RUNNER_TEMP` set,
`devenv shell` for `nix develop`), at the final tree:

```
INFO: seed corpus: files: 3 min: 10b max: 26b total: 60b rss: 35Mb
Done 1000000 runs in 25 second(s)
INFO: seed corpus: files: 13 min: 18b max: 3552b total: 15391b rss: 36Mb
Done 5000000 runs in 13 second(s)
real 0m44.720s; step rc=0; artifacts: 0
```

**The tests fail against what they guard** (each change made, the
scootbar unit tests run, the change reverted):

- A planted finding (`fuzz::format` asserting on inputs containing
  `BOOM`, and `regressions/format/planted` = `\0BOOM`): the fuzz step
  exits 1 with `::error title=scootbar fuzz finding (format)::crash-722dab7b…`
  and `base64: AEJPT00=`; the stable
  `the_format_corpus_and_every_past_finding_replay_cleanly` fails.
- The includes rotting: `use crate::paint::Span as _Rot;` added to
  `tzif.rs` builds in scootbar (`Finished`) and fails the fuzz crate
  (`error[E0432]: unresolved import crate::paint`), which only the new CI
  step compiles.
- The clock panicking on `POLLERR`: 2 of 156 fail,
  `every_registered_module_honours_the_contract` and
  `an_error_or_hang_up_on_the_timer_leaves_it_ticking`.
- The clock's tests no longer naming the harness (`Harness :: new`, which
  still compiles): 1 of 156 fails, `every_registered_module_has_harness_tests`.
- The text baseline floored instead of rounded (`text.rs`,
  `Metrics::baseline`): **only** `snapshots::tests::bar_clipped_at_1_25x`
  fails; the 155 others pass, every existing pixel, text and render test
  among them (their sizes put the baseline where flooring and rounding
  agree: 45 at a 50-pixel em in 60 rows). That is what the snapshots add.

**The snapshots were looked at** before being committed, scaled up as PNGs
(`bar-1x`, `bar-1.5x`, `bar-clipped-1.25x`, `fills`): the digits, `:`,
the `.notdef` block and the icon where the scene puts them, the muted
class dimmer, and the clipped bar losing its center section and its right
one cut, as the layout's rule says.

**The macOS stub**, which this container cannot build (no Darwin standard
library), through the same `cfg(not(target_os = "linux"))` path on the
dev toolchain's other target: `cargo check -p scootbar --all-targets
--target wasm32-unknown-unknown`, with and without default features, both
`Finished`, compiling scootbar alone (no dependency off Linux, so the job
needs no cache).

**The benchmark**, at `3801c12` with a clean tree (`meta.json`:
`"tree_dirty": false`), release scootbar `28c21890…` (the in-worktree build
of the unchanged code), debug scoot `e8367431…`, 39 min 37 s:

```
$ devenv shell -- python3 scripts/scootbar-bench/bench.py run --out .../bench-m1 \
    --scootbar /tmp/impl-testci-target/release/scootbar \
    --scoot /tmp/impl-testci-target/debug/scoot --scootctl /tmp/impl-testci-target/debug/scootctl
```

Kept as it came out in [`docs/scootbar/bench/m1-clock`](../../bench/m1-clock/table.md)
(`report` re-renders its `table.md` byte for byte; `compare` of it against
itself: 0 regressions); summarized in [the README](../../README.md#m1-like-for-like-by-the-benchmark-script).
The raw idle and switching records:

```
scoot scootbar idle 10 invol 0 win 300.0 | switch 240 2 60.0 cpu 0.13 | screen True fds 5
scoot yambar   idle 20 invol 0 win 300.0 | switch 240 4 60.0 cpu 0.69 | screen True fds 7
scoot waybar   idle 25 invol 0 win 300.0 | switch 240 5 60.0 cpu 1.59 | screen True fds 15
sway  scootbar idle  5 invol 5 win 300.0 | switch 240 1 60.0 cpu 0.15 | screen True fds 5
sway  yambar   idle 15 invol 0 win 300.0 | switch 240 3 60.0 cpu 0.51 | screen True fds 7
sway  waybar   idle 15 invol 0 win 300.0 | switch 240 3 60.0 cpu 1.31 | screen True fds 15
```

Gate: 0 losses on either compositor. The rows M0 also measured reproduce
it: scootbar on scoot 2.0 wakeups a minute, 3,944 KB RSS and 1.65 ms idle
CPU (the clock's record: 2.00, 3,944 KB, 1.96 ms); yambar 4.0, 14.4 MB,
3.5 ms (M0: 4.0, 13.9 MiB, 3.6 ms); Waybar 5.0, 52.5 MiB, 8.2 ms (M0: 5.0,
52.6 MiB, 9.1 ms). Switching workspaces woke none of the three bars (the
60 s switching windows hold exactly a minute's idle wakeups), which is
expected at the clock's scope. Load average 1.81 at the start (tests had
just finished), 0.35 at the end.

The harness's first smoke run found its own bug, fixed before this run:
yambar's and Waybar's first starts timed 1.4 ms and 6.2 ms, a terminal's
redraw taken for the bar's frame (the workspace switch just before, and a
previous bar's zone going away, make the windows commit). Each start now
waits until no client has committed for half a second
(`measure.wait_quiet`, unit-tested).

**Not verified here**: the CI jobs themselves. The workflow runs on pull
requests and on `main`, and no pull request was opened from this branch,
so none of it has run on GitHub: in particular `scootbar-macos` on a real
Mac and the fuzz step on a runner; real hardware of any kind; the
opt-in clock-step test (still [a follow-up](../m1-review-followups.md)).

### Follow-ups, 2026-09-29

The independent review of #329 found gaps, fixed on
`fix/testing-and-ci-followups` rather than filed. The record above is left
as it was; what changed:

- **`scootbar-macos` is gone, and so is the stub it checked.** The bar
  never runs on a Mac (the user, 2026-09-29: "The bar will never run on
  Mac."), and neither does scootbg ("Scootbg only needs Linux too. Scoot
  is there because it'll eventually have scoot for Mac"). Both crates, and
  scootbg-mem, lost their non-Linux stubs and `cfg(target_os = "linux")`
  gates; off Linux, scootbg-mem (which the other two build on) stops the
  build with "run on Linux only" as its first error, and the
  `macos` job's `cargo check` excludes them. Only `scoot` keeps its stub.
  The macOS job's `--no-default-features` check went with it; the
  `scootbar` job's clippy and tests of the smallest build cover that build
  on Linux.
- **The module contract reaches every module.** CI now runs the unit tests
  of every feature set it tests (every module, none, each alone), so a
  module outside `default` is held to the contract; a module unavailable
  on the test machine fails it unless its registry line has a `stand_in`
  the contract drives instead; the harness-tests check reads code, not
  comments or strings; the vacuous `len() <= MAX_TEXT` asserts became a
  check that nothing was cut at the bound; and the registry's sources are
  held to the loop's real capacity (`MAX_POLL`, shared with the daemon),
  not a separate 8.
- **`bench.py report` exits 1** when rule 2 fails (a competitor ahead, or
  a gated row with no scootbar value), as the docs above said it gated.
- **The fuzz step checks its lock file** (`cargo fetch --locked`; cargo-fuzz
  has no `--locked` of its own), so a stale `fuzz/Cargo.lock` fails.

How to run it: [testing.md](../../testing.md).
