# Testing scootbar

How scootbar is tested and how to run it. The harnesses, fuzz targets,
feature matrix and benchmark script grow with
[testing-and-ci](backlog/testing-and-ci.md); this page says what exists.

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

The smallest build (every module left out) is checked too:
`cargo clippy -p scootbar --no-default-features --all-targets -- -D
warnings`.

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
- **The module harness** (`modules/harness.rs`) drives a module as the loop
  does (its sources polled, ready ones handed over) and reads its view:
  the clock's real timer ticking, and a trivial second module (a pipe
  counter, in `modules/tests.rs`) written against the contract to show
  what adding one takes.
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

## Fuzzing

Two `cargo fuzz` targets in `crates/scootbar/fuzz` (its own workspace,
never built by CI): `format`, any bytes as a clock format, and `tzif`, any
bytes as a zone file and as a POSIX TZ string. How to run them, and what a
run found, is in [its README](../../crates/scootbar/fuzz/README.md).

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
