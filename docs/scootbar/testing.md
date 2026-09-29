# Testing scootbar

How scootbar is tested, what CI runs for it, and how it is benchmarked.
The layers, the CI jobs and the benchmark script landed with
[testing-and-ci](backlog/resolved/testing-and-ci-done.md); each later
entry adds its own tests through them.

## Running it

From the repository root, in the dev shell, with `scoot` built beside
scootbar (`cargo build -p scoot -p scootbar`: the end-to-end tests start
it) and sway from the pinned nixpkgs:

```sh
export SCOOTBAR_TEST_SWAY=$(nix build --inputs-from . nixpkgs#sway --no-link --print-out-paths)/bin/sway
SCOOTBAR_REQUIRE_SCOOT=1 SCOOTBAR_REQUIRE_SWAY=1 \
  devenv shell -- cargo nextest run -p scootbar
devenv shell -- cargo test -p scootbar
devenv shell -- cargo clippy -p scootbar --all-targets -- -D warnings
devenv shell -- cargo fmt --check -p scootbar
```

| Variable | Effect |
|---|---|
| `SCOOTBAR_TEST_SCOOT` | the `scoot` binary to run; default, the one beside the test's `scootbar` |
| `SCOOTBAR_REQUIRE_SCOOT` | no `scoot` is a failure, not a skip, and so is no `foot` on `PATH` for the window-placement check (CI sets it) |
| `SCOOTBAR_TEST_SWAY` | the `sway` binary to run; default, `sway` on `PATH` |
| `SCOOTBAR_REQUIRE_SWAY` | no `sway` is a failure, not a skip (CI sets it) |
| `SCOOTBAR_DEBUG_NO_VIEWPORTER` | read by a **debug** `scootbar` only (compiled out of release builds): leaves `wp_viewporter` unbound, as on a compositor without it, so the integer-scale fallback is tested on compositors that have one |
| `SCOOTBAR_BLESS` | rewrites the [snapshots](#snapshots) the tests compare instead of comparing them |

The Cargo-feature matrix, as CI runs it: the default build, the smallest
(every module left out), and each module alone:

```sh
devenv shell -- cargo clippy -p scootbar --no-default-features --all-targets -- -D warnings
devenv shell -- cargo clippy -p scootbar --no-default-features --features clock --all-targets -- -D warnings
```

No test needs a font or time-zone data on the machine: the text tests draw
with a seven-segment font built in code (`src/testfont.rs`), which the
integration tests write to a file and pass as `--font`, and the zone tests
read TZif files and `zdump` output checked in under
`src/modules/clock/fixtures/` (`generate.sh` there remakes them from the
pinned nixpkgs' tzdata).

## What covers what

- **Unit tests**, in a `tests.rs` beside each module: flags and their
  bounds (`cli`, `bar`), colors, scale arithmetic at every step from 0.5 to
  4 against the compositor's rounding (`density`), every ordering of an
  output's events through the pure model, closes and retries included
  (`outputs`), the double buffer's choice of slot (`daemon/canvas`), and
  the canvas's fills and blending (`paint`).
- **Pure drawing** (no Wayland): text measured exactly and drawn digits
  read back from the pixels, crisp and at a fractional size, clipped to
  its span, with the glyph cache filled lazily and held to its bounds, the
  largest font size cached rather than rasterized per draw, and a hostile
  font's oversized glyph skipped instead of allocated (`text`); layout of the three sections, with every small layout checked
  for overlap and overflow (`layout`); and the whole path from views to
  pixels (`render`): a change repaints and damages only its module, a new
  width repaints the bar, a buffer that missed a draw catches up, a state
  class draws in its token.
- **Snapshots**: whole canvases compared pixel for pixel with checked-in
  images; see [Snapshots](#snapshots).
- **The module harness** (`modules/harness.rs`) drives a module as the loop
  does and reads its view, two ways: real events (its sources polled,
  ready ones handed over) and fake ones (`Harness::deliver`: any events on
  any source it added, with no poll: a wake with nothing to read,
  `POLLERR`, `POLLHUP`, `POLLNVAL`). The tests assert on the `Update` it
  returns and the `View`. Through it: the clock's real timer ticking, a
  spurious wake, and an error or hang-up on its timer leaving it ticking;
  and a trivial second module (a pipe counter, in `modules/tests.rs`)
  written against the contract to show what adding one takes. **Every
  module in the registry is held to two tests with no new code**
  (`modules/tests.rs`): it survives every event the loop can hand it on
  each of its sources and still fills a bounded view (or, unavailable,
  says why), and it has a `modules/<id>/tests.rs` that drives it through
  the harness, so a module added without harness tests fails.
- **The clock**: the TZif reader against `zdump` at every transition from
  1900 to 2100 of twelve zones picked for their oddities (half-hour and
  45-minute offsets, a 30-minute DST shift, southern summers, Dublin's
  negative DST, Casablanca, abolished DST, Apia's skipped day), fat files
  and slim ones, 9,824 checks; every prefix of each file and 48,000 random
  corruptions without a panic; the POSIX rule forms tzdata never uses
  (`Jn`, zero-based `n`, negative and past-24-hour times); summer-time
  changes and clock steps shown on the right boundary (`modules/clock`);
  every format specifier and flag, and a property test of 20,000 random
  format strings (parse or refuse, render any instant in any offset, no
  control character, bounded length) (`modules/clock/format`); `TZ` read
  as glibc reads it, and a FIFO, `/dev/zero` or an oversized file refused
  without blocking (`modules/clock/zone`).
- **Fonts**: a font anyone can write read into the heap and unaffected by
  a truncation under it; every unusable file refused with its reason
  (`font`); the mapping rule's premises each tested alone (read-only mount,
  root's, no write bit: `scootbg-mem`'s `file`).
- **On a headless scoot** (`tests/bar.rs`): a bar on each of two outputs
  reserving its height (scoot's `outputs` usable area) and in its color on
  a screenshot; margins, with the zone including the anchored edge's
  margin, the surface exactly the bar, and a real window (`foot`) placed
  beside it; a bottom bar; device-exact buffers at 1.5, 1.25 and 2 with
  the scale changed live by a config reload (checked on the protocol
  trace, since a solid color scaled down looks the same), and the
  integer-scale fallback without a viewporter; zero wakeups
  while idle; exit status 1 when the compositor is killed.
- **The clock on headless scoot and sway** (`tests/clock.rs`): the time
  read back off screenshots on two outputs, in the zone `TZ` names, in the
  12-hour default and in `%H:%M`; a seconds clock ticking on screen, with
  every tick damaging only the clock's span (checked on the protocol
  trace); no wakeup at all between minutes; a real `foot` window placed
  beside the clock bar, then closed and checked gone (no leaked client);
  the refusals for an unusable font; and, on sway, the clock on an output
  plugged in while the bar runs. One test there, `clock_steps_and_summer_time_show_on_time`,
  **sets the system clock** (a step shown at once, summer time starting
  and ending on its minute, through `TFD_TIMER_CANCEL_ON_SET`), so it is
  skipped unless `SCOOTBAR_TEST_SET_CLOCK` is set, as root on a disposable
  machine; it puts the clock back from `CLOCK_MONOTONIC` whatever
  happens. CI does not run it.
- **On a headless sway** (`tests/hotplug.rs`): outputs plugged and
  unplugged at runtime, down to none and back; starting with zero outputs;
  a storm of back-to-back plugs and unplugs with the fd count and the
  mapped buffers checked for leaks; the margin rule again; side margins
  wider than the output (sway's negative width) still drawing a 1-pixel bar.

## Snapshots

`src/snapshots/tests.rs` draws fixed scenes and compares every pixel with
an image checked in beside it: span fills (overlapping, empty, clipped at
the output's edge and at the end of `u32`) and glyph coverage blended
inside and outside a clip, in color (`fills.ppm`); and a three-section bar
through the whole render path, in the seven-segment test font with an
icon, `.notdef` and three state classes, at scale 1 (`bar-1x.pgm`), 1.5
(`bar-1.5x.pgm`) and 1.25 too narrow for its modules, so they clip
(`bar-clipped-1.25x.pgm`). The pixel tests beside each module assert what
they mean; these pin everything else in the picture (the antialiasing,
where a fractional scale lands an edge), so any change to it is seen.

The images are ASCII PGM (gray) and PPM (color), one image row per line:
any image viewer opens them, and a change is a readable diff in review.
After a change meant to alter the pixels:

```sh
SCOOTBAR_BLESS=1 devenv shell -- cargo test -p scootbar snapshots
```

rewrites them; look at the diff or the images before committing. Without
`SCOOTBAR_BLESS` a missing image fails rather than being written, so CI can
never bless one. A failure writes what was drawn to
`$TMPDIR/scootbar-snapshots/NAME.actual.pgm` and prints both images as
text. An image no test compares fails too (`no_snapshot_is_left_over`).
Rounded rectangles have no scene yet: the canvas has no rounded shape
until [appearance](backlog/appearance.md) adds one, with its snapshots.

## Fuzzing

Two `cargo fuzz` targets in `crates/scootbar/fuzz` (its own workspace):
`format`, any bytes as a clock format, and `tzif`, any bytes as a zone file
and as a POSIX TZ string. What each checks is written once, in
`src/modules/clock/fuzz.rs`, which the fuzz crate compiles by `#[path]`
beside the two parsers; a stable test replays the seed corpus
(`fuzz/corpus/`) and every past finding (`fuzz/regressions/`) through the
same functions on every `cargo test`. CI builds and runs both targets on
every scootbar change for a fixed budget (below). Longer runs, and what
they found, are in [its README](../../crates/scootbar/fuzz/README.md).

Later parsers each land with their target: the config file (M3), the
`exec` and `push` JSON (M4), a hand-rolled D-Bus message parser (M6).

## CI

The `scootbar` path filter in `.github/workflows/ci.yml`'s classify job
(`crates/scootbar/`, `crates/scootbg-mem/`, `scripts/scootbar-bench/`)
starts these on a pull request, and nothing of the compositor's unless
the change reaches it too (the crate's `Cargo.toml` does):

| Job | What it runs |
|---|---|
| `scootbar` | `fmt --check`; the benchmark harness's unit tests; clippy `-D warnings` on the default build, `--no-default-features`, and each module alone (the modules read from `Cargo.toml`, so a new one joins with no workflow change); `cargo nextest run` and `cargo test` (both: `CLAUDE.md` says why); no `libc` crate, and an `ldd` check that the release binary links only libc, libm and libgcc_s (no libEGL, libgbm or libwayland); both fuzz targets for a fixed budget, 1,000,000 runs of `format` and 5,000,000 of `tzif` from seed 1 (about 35 s), a finding's input printed in base64 |
| `scootbar-integration` | the integration tests on headless scoot and sway, with `SCOOTBAR_REQUIRE_SCOOT` and `SCOOTBAR_REQUIRE_SWAY` so a missing compositor fails instead of skipping; also on a compositor-only change |
| `scootbar-macos` | `cargo check -p scootbar --all-targets` of the macOS stub, with and without modules, when the bar changed and the compositor did not (the `macos` job's workspace check covers every other case) |

`nix-build.yml` builds `.#scootbar` on `main` only
([nix-package](backlog/resolved/nix-package-done.md)).

## Benchmark

`scripts/scootbar-bench/bench.py` measures scootbar against yambar and
Waybar for the [resource ratchet](backlog/lightest.md). It reuses
`scripts/scootbg-bench`'s runner (the headless compositor and its
protocol trace, per-run cgroup CPU accounting and `/proc` readings, the
Size row, the noise rule) and adds the bars, the stage and the rows.

```sh
devenv shell -- cargo build --release -p scootbar
devenv shell -- cargo build -p scoot -p scootctl     # any profile
devenv shell -- python3 scripts/scootbar-bench/bench.py run --out /tmp/sbar \
  --scoot target/debug/scoot --scootctl target/debug/scootctl
python3 scripts/scootbar-bench/bench.py report /tmp/sbar            # tables and gate 2
python3 scripts/scootbar-bench/bench.py compare /tmp/sbar /tmp/old  # gate 1, exit 1 on a regression
python3 -m unittest discover -s scripts/scootbar-bench -p 'test_*.py'
```

It runs as root (or with the right to make a cgroup), and takes about 40
minutes with the defaults (`--rounds 5 --settle-secs 30 --idle-secs 300
--switches 240 --switch-hz 4`, both compositors, three bars one after
another). `--compositors`, `--bars` and the timings narrow it.

**The stage** is M0's: one 1920×1080 output of headless scoot or sway
(pixman), a private session bus, fontconfig seeing DejaVu only, and two
`foot` windows on two workspaces. **Each bar shows exactly the milestone's
scope** (`--scope`, `clock` at M1): a 26-pixel top bar, DejaVu Sans at 14
pixels, `#1e1e2e` behind `#cdd6f4`, `%a %d %b %H:%M` on the right, minute
updates. Competitors come from the pinned nixpkgs.

**The rows**, per bar and compositor:

- *Startup to first frame* (median of `--rounds`): from just before `exec`
  to the bar's first buffer commit as the compositor received it
  (`WAYLAND_DEBUG=server`), each start held back until no client has
  committed for half a second so a window's redraw is never taken for the
  bar's frame. The compositor printing its whole protocol costs every
  bar's round trips alike, so these times are larger than M0's (which
  read the bar's own debug output): compare them within a run or with
  another run of this script, not with the tables in the
  [README](README.md#baselines).
- *Idle*: a fresh start, its first frame awaited, a fixed settle, then a
  window: RSS, PSS, heap (`RssAnon`) and peak (`VmHWM`) at its end;
  **wakeups** as voluntary context switches over every thread (the
  ratified target counts these); CPU from the run's cgroup, which keeps
  exited children's time too. A screenshot then checks the bar's color is
  at the top of the output, so a bar that failed to draw fails its row.
- *Switching*: straight after, `--switches` workspace switches at
  `--switch-hz` between the two windows' workspaces, the CPU and wakeups
  over them. A bar showing no workspaces still gets the row: what the
  compositor's churn costs a bystander.
- *Size*: the stripped binary plus its non-glibc `ldd` closure, gated; the
  bare executable beside it, not gated (the ruling in
  [lightest](backlog/lightest.md#decisions)).
- scootbar's own lines of Rust and direct dependencies, reported.

**The gates** use scootbg's noise rule (a side wins only by more than the
largest of 5%, the two sides' combined spread, and the unit's floor):
`report` lists every row on which a competitor beats scootbar (rule 2),
and `compare` every row on which scootbar is worse than an earlier run
(rule 1). The first run, M1's, is in
[testing-and-ci's record](backlog/resolved/testing-and-ci-done.md#evidence);
later milestones compare against it.

## Not covered

- **Real hardware**: every run above is headless and pixman. Suspend and
  resume in particular: the kernel reports a resume to the clock's timer
  as a clock step (M0 read it: `timekeeping_resume` calls
  `timerfd_resume`); the step path's arithmetic is unit-tested and M0's
  spike measured the kernel side, but no suspend was run, and the opt-in
  step test above has not been run on this code (see the clock's record).
- **A compositor that closes a bar surface**: the retry-once-then-give-up
  path is unit-tested on the model only; neither compositor here closes
  a bar that is still on a live output.
