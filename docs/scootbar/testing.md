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

The Cargo-feature matrix, as CI runs it: clippy on the default build, the
smallest (every module left out) and each feature alone (every feature but
`default` is in the matrix: the modules, and `icon-image`, the PNG decoder,
which is not a module and is off by default); unit tests on the default
build, everything at once (`--all-features`), the smallest and each feature
alone. The module contract below walks the
registry of the build it is compiled in, so a module left out of
`default` is held to it only by these:

```sh
devenv shell -- cargo clippy -p scootbar --no-default-features --all-targets -- -D warnings
devenv shell -- cargo clippy -p scootbar --no-default-features --features clock --all-targets -- -D warnings
devenv shell -- cargo clippy -p scootbar --no-default-features --features icon-image --all-targets -- -D warnings
devenv shell -- cargo nextest run -p scootbar --bin scootbar --all-features
devenv shell -- cargo nextest run -p scootbar --bin scootbar --no-default-features
devenv shell -- cargo nextest run -p scootbar --bin scootbar --no-default-features --features clock
devenv shell -- cargo nextest run -p scootbar --bin scootbar --no-default-features --features icon-image
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
  (`modules/tests.rs`), in every build CI tests (see the feature matrix
  above):
  - *The contract*: started as the bar starts it, it survives every event
    the loop can hand it on each of its sources, and before and after each
    it fills a view with no control character and nothing cut at the
    view's bound (`MAX_TEXT`: the bound is for untrusted text, so a
    module's own output cut there is a bug). Unavailable, it must say why.
    The registry's modules together poll no more fds than the loop has for
    them (`MAX_POLL`, less the Wayland connection's), since a layout
    places each once.
  - *Stand-ins*: a module unavailable on the test machine (a battery
    module where there is no battery) would otherwise skip all of that, so
    it fails the contract unless its registry line has a `stand_in`
    (`Spec::stand_in`, compiled in tests only): the module started as if
    its probe had found what it looks for, a fake device or a fixture.
    The contract then drives the stand-in too, on every machine. A module
    available anywhere tests run (the clock: a `timerfd`) needs none. What
    it cannot check is that a stand-in is the real module: any
    `Box<dyn Module>` satisfies it, so review is what keeps it honest.
  - *Harness tests*: it has a `modules/<id>/tests.rs` whose code calls the
    harness (`Harness::new(` or `Harness::start(`; comments and string
    literals are skipped, so a mention is not a call) and a
    `#[cfg(test)] mod tests;` in `modules/<id>/mod.rs`, so the file is
    compiled. A module added without harness tests fails. It cannot tell a
    call that runs from one in a dead function, so a test that never
    asserts anything is also review's to catch.
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
  beside it; a floating rounded bar whose cut corners show the desktop
  and a half-opaque one blended over it, on screenshots; a bottom bar; device-exact buffers at 1.5, 1.25 and 2 with
  the scale changed live by a config reload (checked on the protocol
  trace, since a solid color scaled down looks the same), and the
  integer-scale fallback without a viewporter; zero wakeups
  while idle; exit status 1 when the compositor is killed.
- **Layers, the zone and hiding on a headless scoot**
  (`tests/visibility.rs`): every layer on both edges, with and without the
  zone, drawn along its edge and reserving its height or nothing; the layer
  and the zone in the protocol requests (pixels cannot tell `bottom` from
  `top` on an empty desktop); `msg hide` releasing the zone and every
  `wl_shm` mapping of the daemon, and `show` bringing both back; forty
  concurrent `toggle`s settling on the net result; a reload keeping a hidden
  bar hidden; a real window (`foot`) reclaiming the space and returning to
  its rectangle with no intermediate one, sampled every few milliseconds.
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
- **The bar's shape on headless scoot** (`tests/appearance.rs`, and the
  circle pill in `tests/workspaces.rs`): a click in the cut corner of a
  rounded bar reaches the window behind it while a click on its flat part
  does not, and a square bar swallows both (the negative control: with the
  input region left unset the test fails, checked); the first module's ink
  sits exactly `radius - padding / 2` further in on a rounded bar than on
  a square one; a module's `margin` moves it by exactly that much; and a
  `pill-shape = "circle"` pill is round on a screenshot and a click on
  another workspace's digit still switches to it. Pure: the input region's
  rectangles (`region`: every pixel the paint gives any coverage is
  clickable, the four corners mirror, no overlap, no underflow at any size),
  the layout with margins and an edge inset (`layout`), the separators and
  the corner clearance through the render path (`render/tests/spacing.rs`),
  the pill's fill and the pill's geometry (`paint`, `modules/workspaces/pill`,
  and `hit_target` for the click), and every config bound
  (`config/spacing_tests.rs`).

- **Pointer input** (M4's [pointer-and-interactions](backlog/resolved/pointer-and-interactions-done.md)),
  pure: the pointer state machine in every order it can see (`pointer`:
  a click fires on release and only over the module it was armed on, a
  release after the pointer left or slid off or on another output is
  nothing and leaves nothing armed, a chord is no click, other buttons and
  a release with no press are ignored, a press over nothing arms nothing;
  one notch is one step whether the wheel sends `axis`, `axis_discrete`,
  `axis_value120` or all of them in either order, smooth scrolls add up
  and drop their remainder on a reversal, `axis_stop` and leave, ten
  thousand events are one action per 16 ms frame with the steps capped
  and carried, hostile values and a clock running backwards do not
  overflow, touch and the keyboard never get a pointer); hover in the
  render path (`render/tests/hover.rs`: a hover repaints and damages only
  the span left and the span entered, in either buffer, on one output
  only, never bumps a module's revision, follows a module that grows or
  vanishes under the pointer, and a press is routed by the layout on
  screen until the next draw); the config keys (`config/binding_tests.rs`:
  every action kind, every refusal naming its key, the bounds on an `exec`
  line); `action` (what reaches a module, the effects, a failing command);
  the spawner (`spawn`: no shell, reaped with no zombie through the
  pidfd and through the fallback, the cap of 8 and the slot that frees, a
  flood of short commands, process group, no inherited descriptor, with a
  control that shows the check sees a leaked one); and scoot's `quit` (`scoot`: the
  request is what `scoot-ipc` encodes, a scoot that closes, refuses,
  hangs, is absent or has a full queue, an endless reply).
- **Pointer input on a headless scoot** (`tests/pointer.rs`, through scoot's
  own `click`, `pointer_move`, `pointer_button` and `scroll` injection):
  each button runs its own binding; a press off the module, or a release
  after the pointer left it, is no click (and breaking that fails the test,
  checked); a four-hundred-event scroll flood is a handful of commands, at
  most one per frame, with no zombie left; a command launched by a running
  bar holds none of the descriptors the bar opens (the bar holds real ones:
  Wayland and control sockets, the lock, memfds, the timerfd; what the
  test process itself was handed by its launcher is allowed through, and the
  bar holds no descriptor without close-on-exec beyond those) and the bar's own count
  does not move; short commands leave no zombie and a hung one is capped
  at eight, said, and its slot reused; a failing command is one line, not
  thirty; hover tints the clock on a screenshot and only while the pointer
  is on it, and a clock with no binding is never tinted; the pointer is
  taken only while a binding needs it and released when a reload removes
  it (on the protocol trace); a reload with a misspelled action is refused
  by key and changes nothing, and one with a good binding takes the
  pointer. Touch cannot be injected, so it is covered by the pure test.

## Snapshots

`src/snapshots/tests.rs` draws fixed scenes and compares every pixel with
an image checked in beside it: span fills (overlapping, empty, clipped at
the output's edge and at the end of `u32`) and glyph coverage blended
inside and outside a clip, in color (`fills.ppm`); and a three-section bar
through the whole render path, in the seven-segment test font with an
icon, `.notdef` and three state classes, at scale 1 (`bar-1x.pgm`), 1.5
(`bar-1.5x.pgm`) and 1.25 too narrow for its modules, so they clip
(`bar-clipped-1.25x.pgm`); and the rounded and translucent bar, as the
alpha plane (white where the bar is, so the cut shows) at scale 1 with
radius 6 (`bar-rounded-1x-alpha.pgm`) and at 1.5 with half opacity
(`bar-rounded-1.5x-alpha.pgm`), the color plane of a translucent bar
with text blended over it, premultiplied (`bar-translucent-1x.pgm`), the
separators and a margin (`bar-separated-1x.pgm`), and the workspaces pill
as a pill, a circle around one digit and a circle widened around two,
at 1 and at 1.5 (`workspaces-pill-1x.pgm`, `workspaces-circle-1x.pgm`,
`workspaces-circle-two-digits-1x.pgm` and the `-1.5x` ones); and, from
`src/snapshots/icons.rs`, a path icon (a "home" and a ring drawn with arcs, in
two state tokens) with text at 1 and 1.5 (`icon-path-1x.pgm`,
`icon-path-1.5x.pgm`), one alone at 1.25 (`icon-path-only-1.25x.pgm`), and a
PNG icon (a soft-edged disc, gray and alpha) at 1 and 1.5 (`icon-image-1x.pgm`,
`icon-image-1.5x.pgm`, compared in a build with `icon-image`). The
pixel tests beside each module assert what
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
The workspaces scenes are drawn by `modules/workspaces/tests.rs` through
the same `check`.

## The appearance hardware test

`scripts/scootbar-appearance-hw-test.sh` is what real hardware adds to the
headless tests above: the four looks of [appearance](backlog/resolved/appearance-done.md)
(flush-opaque, rounded-opaque, rounded-translucent, floating), each on a real
compositor, in one run that ends in a PASS/FAIL/INFO table. Nothing in the
CI runs it; **the maintainer runs it on hardware, and the numbers go into
the [resource ratchet](backlog/lightest.md#appearance-looks-flush-against-floating)**.

```sh
cargo build --release -p scoot -p scootctl -p scootbar
# On the machine: Ctrl+Alt+F3, log in, cd to the checkout, and stay on that VT
SCOOTBAR_HW_MODE=--tty SCOOTBAR_HW_OUT=/tmp/sb-appearance-tty scripts/scootbar-appearance-hw-test.sh
```

`SCOOTBAR_HW_MODE` is `--headless` (the default, no hardware: how the script itself is
rehearsed), `--nested` (run inside a host compositor) or `--tty` (a real
display: the run that counts, about four minutes; the script says why the
VT must be kept and marks a run whose compositor was paused). `SCOOTBAR_HW_OUT` is a new
directory (a run already in it is refused, `SCOOTBAR_HW_OVERWRITE=1` replaces it). The
header of the script lists the other settings (`SCOOTBAR_HW_GAP`, `SCOOTBAR_HW_RADIUS`, `SCOOTBAR_HW_IDLE_SECS`,
`SCOOTBAR_HW_REDRAWS`, ...; `SCOOTBAR_HW_OUTPUTS=eDP-1` for one bar on a box with several displays (the default puts a bar on each), `SCOOTBAR_HW_FONT=/path/to/DejaVuSans.ttf` on a box whose fonts live outside the usual system paths, such as NixOS, or the bars run with no clock) and the prerequisites: release builds, `python3` (standard
library only: the PNG decoding and the pixel checks), `foot` for the click
check. Exit status 0 with no FAIL, 1 with a FAIL, 2 for a setup problem (no
binary, a busy seat, the compositor never came up), which is not a result.

Per look it does, in order:

- **Pixels**, on a `scootctl screenshot` of the output (no pointer): the four
  corners of a rounded bar show the desktop and a flush square one shows the
  bar's color there; the top edge just past a corner, the left edge's middle
  and the body are the bar's; a translucent bar's body is the blend of the
  bar and the desktop within six levels; a floating bar has the desktop in
  its margin and at the screen's corner. Every sampled pixel is in
  `pixels.tsv`.
- **The zone**: the usable area's top is the bar's height (flush) or the
  height plus the margin (floating).
- **The protocol**: in a run of its own with `WAYLAND_DEBUG=1`, a square bar
  makes no `set_input_region` request and a rounded one makes one; the
  first `damage_buffer` is the bar's own size (a floating bar's is narrower by
  its side margins).
- **Idle**: after settling, `SCOOTBAR_HW_IDLE_SECS` of nothing: scootbar's context
  switches (the wakeups: at most 8 allowed for a clock's tick and the
  compositor's release), its CPU jiffies (at most 2) and its RSS, and the
  compositor's jiffies.
- **A whole-bar redraw** repeated `SCOOTBAR_HW_REDRAWS` times (`scootbar msg reload`
  re-places every output, which draws the whole bar and hands the compositor a
  new buffer): the CPU jiffies of scootbar and of scoot. Each look is compared
  with flush-opaque at the end.
- **A cursor sweep** along the bar, paced below the display's refresh rate so
  every move is a frame, for `SCOOTBAR_HW_SWEEP_SECS`: scoot's CPU per move, which is the
  compositor's cost of the look under the cursor (blending a translucent
  bar, and the opaque region an opaque one declares).

After the four looks, **the click check**: a 120-high bar with radius 60 that
reserves no space, over two `foot` windows. A click on the bar's flat part must
not move focus; a click in the left window's corner, under the bar's cut
corner, must move focus to that window, and the same click under a square bar
of the same size must not (the control that shows the harness measures the
bar). `SCOOTBAR_HW_EXPECT_INPUT_REGION=ignored` states the opposite expectation for a
compositor that ignores input regions; scoot honors them.

**Every setting is namespaced `SCOOTBAR_HW_*`**, because generic names
collide with the environment (devenv's stdenv exports `SIZE=size`, which broke
an earlier `SIZE` knob); `SCOOTBAR_HW_SIZE` must be `WIDTHxHEIGHT`.

**`--tty` guards**: it is refused (exit status 3, nothing written, not even
the output directory) when `WAYLAND_DISPLAY` or `DISPLAY` is set, that is,
when run from inside a graphical session, unless `SCOOTBAR_HW_TAKE_SEAT=1`.
At start-up it prints how to get back: a VT switch (`Ctrl+Alt+F<n>`, the VT
of your session) is the only abort, since once scoot owns the keyboard
`Ctrl+C` reaches scoot's focused `foot`, not the script. The refusals are
pinned by `tests/hw_script.rs`.

**What only makes sense on hardware**: the CPU and RSS numbers (the headless
compositor presents nowhere, and debug builds are marked INFO, not for the
ratchet), the cursor sweep's compositor cost, and the whole point of `--tty`,
which is that the corners and the blend are what a real display shows. The
pixel, zone, protocol and click checks are the same everywhere and pass on
`--headless` and `--nested` (rehearsed for this entry, see the results in
[appearance](backlog/resolved/appearance-done.md#landed-and-remaining)). The CPU
differences are small next to the 10 ms jiffy: a difference of one jiffy over
a run is noise, and the table says so; a change is worth quoting when it
holds across two runs.

**What to send back**: the `SCOOTBAR_HW_OUT` directory, or at least `summary.tsv`,
`pixels.tsv`, `results.txt` and `environment.txt` (the commit and the exact
binaries, the cache key of the numbers), plus any FAIL line and, for a
failing pixel, its screenshot (`SCOOTBAR_HW_OUT/<look>.png`). Paste `summary.tsv` into the
ratchet's appearance table.

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
| `scootbar` | `fmt --check`; the benchmark harness's unit tests; clippy `-D warnings` on the default build, `--no-default-features`, and each module alone (the modules read from `Cargo.toml`, so a new one joins with no workflow change); `cargo nextest run` and `cargo test` of the default build (both: `CLAUDE.md` says why), then `cargo nextest run` of the unit tests with every module, with none, and with each alone; no `libc` crate, and an `ldd` check that the release binary links only libc, libm and libgcc_s (no libEGL, libgbm or libwayland); both fuzz targets for a fixed budget, 1,000,000 runs of `format` and 5,000,000 of `tzif` from seed 1 (about 35 s), a finding's input printed in base64, after `cargo fetch --locked` of the fuzz workspace (cargo-fuzz has no `--locked`), so a stale `fuzz/Cargo.lock` fails |
| `scootbar-integration` | the integration tests on headless scoot and sway, with `SCOOTBAR_REQUIRE_SCOOT` and `SCOOTBAR_REQUIRE_SWAY` so a missing compositor fails instead of skipping; also on a compositor-only change |

The bar never runs on a Mac, and does not build there (neither does
scootbg): the `macos` job's `cargo check` leaves both out, with
scootbg-mem, and no change to them alone starts it.

`nix-build.yml` builds `.#scootbar` on `main` only
([nix-package](backlog/resolved/nix-package-done.md)).

## Benchmark

`scripts/scootbar-bench/bench.py` measures scootbar against yambar and
Waybar for the [resource ratchet](backlog/lightest.md), and against ironbar
and ashell as informational extras (below). It reuses
`scripts/scootbg-bench`'s runner (the headless compositor and its
protocol trace, per-run cgroup CPU accounting and `/proc` readings, the
Size row, the noise rule) and adds the bars, the stage and the rows.

```sh
devenv shell -- cargo build --release -p scootbar
devenv shell -- cargo build -p scoot -p scootctl     # any profile
devenv shell -- python3 scripts/scootbar-bench/bench.py run --out /tmp/sbar \
  --scoot target/debug/scoot --scootctl target/debug/scootctl
# --scope clock (the default, M1's) or clock-workspaces (M3's);
# --bars scootbar,yambar,waybar,ironbar,ashell adds the informational pair
# (the default is the first three); --scootbar PATH
# and --scootbar-source TREE measure a binary built elsewhere, such as an
# earlier milestone's for a like-for-like compare
python3 scripts/scootbar-bench/bench.py report /tmp/sbar            # tables and rule 2, exit 1 on a loss
python3 scripts/scootbar-bench/bench.py compare /tmp/sbar /tmp/old  # rule 1, exit 1 on a regression
python3 -m unittest discover -s scripts/scootbar-bench -p 'test_*.py'
```

It runs as root (or with the right to make a cgroup), and takes about 40
minutes with the defaults (`--rounds 5 --settle-secs 30 --idle-secs 300
--switches 240 --switch-hz 4`, both compositors, three bars one after
another). `--compositors`, `--bars` and the timings narrow it.

**The stage** is M0's: one 1920×1080 output of headless scoot or sway
(pixman), a private session bus, fontconfig seeing DejaVu only, and two
`foot` windows on two workspaces. **Each bar shows exactly the milestone's
scope** (`--scope`): a 26-pixel top bar, DejaVu Sans at 14 pixels,
`#1e1e2e` behind `#cdd6f4`, `%a %d %b %H:%M` on the right, minute updates.
Competitors come from the pinned nixpkgs. The scopes:

| Scope | Milestone | scootbar | Waybar | yambar |
|---|---|---|---|---|
| `clock` | M1 | `--right clock` | `clock` on the right | `clock` on the right |
| `clock-workspaces` | M3 | `--left workspaces --right clock` | `ext/workspaces` (scoot) or `sway/workspaces` (sway) on the left, the clock on the right | on sway, the `i3` module (it speaks sway's IPC) on the left, the clock on the right; **on scoot, nothing** |

| Scope | ironbar 0.19.0 (informational) | ashell 0.10.0 (informational) |
|---|---|---|
| `clock` | `clock` in `end` | `Tempo` (its clock) in `right`, `left` and `center` empty |
| `clock-workspaces` | on sway, `workspaces` in `start` and `clock` in `end`; **on scoot, nothing** (no ext-workspace-v1) | `Workspaces` in `left`, `Tempo` in `right`, on both |

**yambar cannot show workspaces on scoot**: 1.11.0 has no ext-workspace-v1
module (its workspace modules are `i3`, `river` and `dwl`). The harness does
not run it there and does not invent a number: the column reads "cannot show
this scope", `meta.json` records why (`cannot_show`), and `report` ends with
`NOT COMPARED on scoot: yambar ...` and counts it as **not passed** (exit 1),
since the rule does not say what to do when a competitor cannot show the
scope. That is the conservative reading of "a gate that cannot be judged has
not been passed"; whether it should instead be waived for that pair is the
maintainer's call, not the script's. On sway yambar is compared in full.
**ironbar and ashell are informational** (`--bars` opts them in; the default is
scootbar, yambar and Waybar, the ratified competitors). Their columns are
measured and shown (`meta.json` marks them `informational`), a row either wins
is printed under `Informational, not gated` as a finding, and the gate
never counts them: not as a loss, not as "no scootbar value", and not as
"not compared". Promoting them into the rule is the maintainer's call. They run
only as the nixpkgs binaries (ashell is GPL-3.0-or-later, ironbar MIT; no
code or config of theirs is copied), each with the config keys the common look
needs and nothing more:

- **ironbar 0.19.0** (`-c ironbar.json -t ironbar.css`): `position`,
  `height: 26`, the `workspaces` module in `start` and `clock` (the format) in
  `end`; the CSS sets DejaVu Sans 14 px and `#bar`'s background and color. It has no
  ext-workspace-v1 support, so at `clock-workspaces` it is **not run on scoot**
  ("cannot show this scope", as yambar) and runs on sway through its sway
  support.
- **ashell 0.10.0** (`-c ashell.toml`): `[modules]` with `Workspaces` left and
  `Tempo` (its clock; 0.10.0 has no `Clock` module, and an unknown table is
  ignored with a warning) right, `center = []` (its default has a window title
  there), `[tempo] clock_format`, `[appearance] font_name`/`text_color`,
  `[appearance.bar] surface = "solid"` and `[appearance.background_color]
  base`. Its bar height and font size are not options, so they are its own. It
  shows workspaces on both compositors.
- Neither needed software-rendering variables on the Asahi box's headless
  sessions (checked: no DRM device open in `/proc/PID/fd`); a box where they
  failed to start would need `WGPU_BACKEND`/`LIBGL_ALWAYS_SOFTWARE`, which
  could change their cost, and the run would have to say so.

A scope's bar must also draw: the screenshot check after the idle window
crops each placed part (a 300-pixel span on the left, center or right of
the bar) and fails the run if it shows a single color, so a workspaces module
that drew nothing (no protocol, no socket) is a failed run and not a cheap row.

**The machine** is recorded, since numbers from a laptop depend on it: every
record carries the cpufreq governor and current, policy-maximum and
hardware-maximum frequencies of each CPU, every hwmon temperature, the power
supplies (mains online, battery) and the load average, before and after it
(`hw_start`/`hw_end`), and `meta.json` has them at the start and end of the
run plus the CPU model and page size. `report` summarizes them (a policy cap
below the hardware maximum, mains lost, the frequency range, the temperature
range). The Apple M2 exposes no CPU temperature, only NAND, battery, charger
and radio ones, so a throttle there shows only as a cap or a falling clock.

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
  ratified target counts these) counted per thread, so a thread pool that
  comes and goes cannot make a window negative (an exited thread's switches
  are then missed, never subtracted); CPU from the run's cgroup, which keeps
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
largest of 5%, the two sides' combined spread, and the unit's floor), and
each exits 1 when its rule fails: `report` lists every gated row on which
a competitor beats scootbar (rule 2), and every gated row a competitor has
and scootbar does not (its runs failed, or it was not run), since a gate
that cannot be judged has not been passed; `compare` lists every row on
which scootbar is worse than an earlier run (rule 1). Each milestone's run is kept in [`bench/`](bench/README.md),
never overwritten. **A baseline is only comparable when it came from the same
machine**: `bench/m1-clock` was measured on a Claude Code cloud sandbox VM (4-vCPU Xeon, not hardware the maintainer owns) with a
debug scoot, so M3's rule-1 check was made **like for like** instead: the
old commit's scootbar is rebuilt on the machine that measures the new one
(`--scootbar PATH --scootbar-source TREE` point the harness at a binary and the
tree it came from), both run the same harness at the old scope, alternating
(A-B-B-A, with a cool-down between) so each side has two runs, and `compare`
is run on each pairing. A row is called a regression only when it
regresses in every pairing; a row that flags in some is reported as noise
with its counts, and a run compared with its own rerun shows what the rule's
false-positive rate is (at 16 KiB pages one page of `RssAnon` is enough to
flag when a row has a single idle sample). M3's is
[`bench/m3-asahi-*`](bench/README.md), and its table is in the
[README](README.md#m3-clock-and-workspaces-on-the-asahi-m2); `m1-clock` stays
as the published M1 record, and its table is in the
[README](README.md#m1-like-for-like-by-the-benchmark-script).

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

## The systemd unit under a real user manager

`scripts/scootbar-unit-test.sh` runs the unit `nixosModules.scootbar` generates
under the real `systemd --user` against a live headless scoot (nothing drawn on
a display, no VT taken): it keeps retrying with no compositor, comes up once
`WAYLAND_DISPLAY` reaches the manager, is restarted after a SIGKILL (and eight in a
row), and stays stopped after a SIGTERM, `scootbar msg kill` and a stop. It builds
the unit from this checkout's committed flake with Nix, loads it into the manager's
*runtime* directory only (nothing persistent), refuses to run if a `scootbar.service`
already exists, and removes everything on exit. Needs Linux with a user manager,
Nix, a built scoot and scootctl, and python3; it prints `RESULT: PASS n FAIL m` and
exits 1 on a FAIL. It is not run in CI (CI has no user manager).
