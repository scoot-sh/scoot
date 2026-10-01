---
title: "Pointer input and interactions: hit-testing, hover, click and scroll actions"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M4"
resolved: "2026-09-30"
---

# Pointer input and interactions — RESOLVED

Resolved 2026-09-30 (draft PR, first of the M4 stack; the ratchet row is
**not** passed, see [the ratchet](#the-ratchet-not-passed): it needs the
maintainer's ruling and a run on real hardware). What landed, the decisions
that were the entry's to make, and the evidence are below the original
entry, which is kept as filed.

Filed 2026-09-29. Serves **daily-drive** (volume scroll, launcher and power
buttons) and **computer use** (every interaction is scriptable).

## What to do

This generalizes the workspaces module's own minimal hit test
([workspaces-module](workspaces-module-done.md), shipped earlier) into the module
trait and the config keys below.

- `wl_pointer` on the bar surface: enter, motion, button, and axis (use
  `axis_value120` / discrete steps so a smooth-scroll touchpad and a wheel
  both move a volume step sensibly).
- Hit-test against per-module rects recorded at layout (a pooled `Vec`
  rebuilt only when layout changes); hover changes damage only the affected
  module's rect.
- `on_input(Input) -> Option<Action>` on the module trait. Config keys on
  every module: `on-click`, `on-right-click`, `on-middle-click`,
  `on-scroll-up`, `on-scroll-down`. A value is a module-defined action
  (`"toggle-mute"`) or `{ exec = [...] }`; modules ship defaults.
- A `scoot = "quit"` action kind that talks to scoot's IPC through an
  optional Cargo feature, so log out needs no process spawn and the bar builds
  without scoot's IPC crate. Measure `scoot-ipc`'s cost first (scootbg avoided
  it). Absent scoot, the action reports it cannot run and does nothing.
- No keyboard interactivity: the bar never takes the keyboard, so it never
  disturbs focus.
- Spawned commands are reaped (scoot's own `spawned-children-never-reaped`
  is the precedent) and never inherit the bar's fds.

## Edge cases to pin

Click during a redraw, a module that disappears under the pointer, scroll
flood at a real device's rate (coalesce to one action per frame), a button
release after the pointer left, a touch device (ignored, stated).

## Done when

Clicks and scrolls run the configured action, hover repaints one rect, and a
scroll flood does not spawn a process per event.

## What landed

[`docs/scootbar/cli.md#pointer-input`](../../cli.md#pointer-input) is the
user-facing reference. In short:

- **The module trait**: `on_input(&self, &Input) -> Option<Action>` (a
  module's own default for a pointer input, pure, so a hit test is a unit
  test) and `invoke(output, action, steps) -> Result<Update, InvokeError>`
  (does it). A configured binding replaces the default and goes through the
  same `invoke`, so a click, a scroll and (next entries) an agent's
  `invoke` take one path. `Spec::actions` lists a module's action names, so a
  typo in the config is refused naming the key. The workspaces module's
  hit test moved into `on_input`; its effect into `invoke`, which gained
  `activate N`, `activate-position N` (what a click means: exact where two
  items show one number), `previous` and `next`.
- **Config**: `on-click`, `on-right-click`, `on-middle-click`,
  `on-scroll-up`, `on-scroll-down` on `[clock]` and `[workspaces]`; a value
  is a module action, `{ exec = [...] }` or `{ scoot = "quit" }`.
- **`crates/scootbar/src/pointer.rs`**: the pure state machine. A click fires
  on **release**, only over the module the press armed (a release after the
  pointer left or slid off is nothing); a chord is no click. Scroll is
  counted in steps without double-counting `axis` beside `axis_discrete` or
  `axis_value120`, and **released at most once per 16 ms frame** with the
  steps that piled up (capped at 32), the loop sleeping only until the frame
  is due, and never at all while idle.
- **Hover** (`render.rs`): per output, in the scene and its records, never a
  module's revision (which every output shares). Only the span left and the
  span entered are repainted and damaged, in either buffer. The tint is the
  existing `accent` token.
- **`crates/scootbar/src/spawn.rs`**: `exec` commands run with no shell, on
  `/dev/null`, in their own process group, with none of the bar's
  descriptors, **reaped through a pidfd the loop polls** (no signal handler,
  no thread, no timer; a fallback poll timeout only on a kernel without
  pidfds), at most 8 at once.
- **`crates/scootbar/src/scoot.rs`**: `{ scoot = "quit" }`.
- **A pointer only when needed**: the bar binds the seat and asks it for a
  `wl_pointer` only while a placed module has a binding or a default (the
  workspaces click). A clock-only bar with no bindings takes none.
- **The Nix user unit gets `KillMode=process`**, which this makes necessary:
  a launched app is the bar's child, and the default `control-group` kills it
  with the bar on every restart (a config change restarts the unit).
  Checked on the VM's `systemd --user`: a transient unit's child is gone
  after `stop` with the default and survives with `KillMode=process`.

## Decisions

1. **`scoot = "quit"` is hand-written, not `scoot-ipc`, and is not a Cargo
   feature.** The entry said to measure `scoot-ipc` first. Release build,
   aarch64, stripped, on the dev VM:

   | build | bytes | |
   | --- | --- | --- |
   | `main` (`3211551bd`) | 1,249,984 | |
   | this PR, no `scoot-ipc` | 1,315,520 | +65,536 |
   | this PR with the `scoot-ipc` client linked (a feature, built to measure) | 1,446,592 | **+131,072 (+10.0%)** over this PR |
   | this PR with a hand-written request (what shipped) | 1,315,520 | `.text` +2,496 bytes, no file-size step |

   (`.text` of the three builds: `0x0e2328`, `0x0ef8c8`, `0x108368` linked,
   `0x0f0288` hand-written.) The request is one fixed line, so it is written
   by hand with a non-blocking connect, a 250 ms timeout each way and a bound
   on the reply; a `scoot-ipc` **dev-dependency** encodes the same request in
   a test, so the wire cannot drift unnoticed. **A maintainer call**: the
   entry's wording was "an optional Cargo feature"; with the crate left out
   the feature has nothing to gate, so it is always built.
2. **An `exec` binding runs once per coalesced scroll and is given no step
   count.** A `SCOOTBAR_STEPS` variable was built and removed: setting an
   environment variable on a `Command` costs about 8 KB of binary (the
   `BTreeMap` it carries), for a number only a flood of events changes. A
   module action takes the count.
3. **Hover draws in `accent`, on by default for a module with a binding.**
   There is no `hover` token yet
   ([appearance-followups](../appearance-followups.md)). **A maintainer
   call**: that entry says a new look is off by default; this one is a
   tint a bound module gets, with no option, costing a repaint of one span.
4. **Clicks fire on release.** A press-fires design is simpler and what the
   workspaces module did; release-fires is what makes "a release after the
   pointer left" a non-event, and costs nothing.
5. **A click means the item it landed on** (`activate-position`), because a
   first draft routed it through `activate N` and two items can show one
   number; review of the draft found it, a test pins it.

## The ratchet: not passed

Measured on the dev VM (aarch64, 6 vCPUs, `scoot --headless` from
2026-09-28, release scootbar), `scripts/scootbar-bench/bench.py run
--compositors scoot --bars scootbar --rounds 3 --settle-secs 20
--idle-secs 120 --switches 40`, two runs of each build alternating, at both
scopes (`--scope clock`, and `--scope clock-workspaces`, M3's), the base
being `main` at `e6ca6e35a` (M3 with the unplaced-workspaces bind fix, #363)
and this PR rebased onto it (`c9361fc4c`'s tree). **These are the dev VM's
numbers, not the ratchet's machine; the run that counts is the maintainer's
on the real hardware, and is still to do.** Every row below is shown, none
waived.

| Row | `main` (2 runs) | this PR (2 runs) | `compare`, run 1 |
| --- | --- | --- | --- |
| **Clock scope** | | | |
| Size, stripped binary + non-glibc closure | 1,383,080 | 1,514,152 | **regressed** (+9.5%) |
| Bare executable (not gated) | 1,249,984 | 1,381,056 | +10.5%, flagged |
| Idle RSS | 3.3, 3.3 MiB | 3.4, 3.4 | same |
| Idle PSS | 1.7, 1.7 | 1.9, 1.9 | **regressed** |
| Idle wakeups per minute | 2, 2 | 2, 2 | same |
| Idle CPU, 120 s window | 0.7, 1.1 ms | 1.0, 0.7 | **regressed** (run 1), same (run 2) |
| CPU while switching workspaces | 0.0, 0.0 ms | 0.2, 0.0 | **regressed** (run 1: 2 wakeups, 0.2 ms), same |
| Startup to first frame | 37.9, 29.0 ms | 37.4, 30.4 | same |
| **Clock and workspaces scope** | | | |
| Idle RSS | 3.3, 3.3 | 3.5, 3.5 | same |
| Idle PSS | 1.7, 1.7 | 1.9, 1.9 | **regressed** |
| Idle CPU, 120 s window | 1.1, 0.7 | 1.1, 1.1 | same |
| CPU while switching workspaces (80 wakeups) | 3.8, 3.8 | 5.2, 4.0 | **regressed** (run 1) |
| Startup to first frame | 29.0, 36.2 | 36.7, 39.5 | **regressed** (run 1) |
| Threads | 1 | 1 | same |
| Lines of Rust / direct dependencies | 25,347 (16,312 outside tests) / 10 | 28,732 (18,429) / 10 | the new code and its tests |

- **The size row is the clearest regression, and part of it is luck.** The
  release binary's `.text` grew 53.7 KB (930,792 to 984,488 bytes, 5.8%) and
  `.rodata`, `.eh_frame` and the rest brought the real growth to about 66 KB
  (5.3%); this aarch64 build lays out segments on 64 KiB boundaries, so the
  file grew by 131,072 bytes (an earlier build of the same code before the
  rebase grew by 65,536). Both are what the ratchet's rule 1 reads.
- **Idle PSS +0.2 MiB is real, and its cause is found**: per-mapping
  `smaps` at the end of the idle window show the whole difference in the
  binary's own executable mapping (`r-xp`, 960 KB resident on `main`,
  1,088 KB before the rebase; page size 4 KiB): the kernel maps 64 KB
  chunks of code around a fault, and the code grew by 55 KB. Where the
  bytes went (`nm`, symbols that grew or are new, 71 KB in all, before the
  rebase): the config's `toml` deserializer for the two tables that gained
  five keys each (about 16 KB), `Command::spawn` and its environment
  plumbing (about 8 KB, before `SCOOTBAR_STEPS` was removed),
  `config::bindings::read` (5.7 KB), and the pointer, hover and action
  code.
- **The CPU rows are not a steady-state regression**: a 1 Hz clock (2
  wakeups a second, 240 in the window) costs the bar 43.4, 40.7 and 41.7 ms
  of CPU over 120 s on `main` (`3211551bd`) and 39.8, 39.7 and 39.4 ms with
  this change (before the rebase); the harness's rows count four wakeups in
  120 s, where cold code dominates, and its switching row swung 2.0 to 4.1
  ms on `main` itself in an earlier set of three runs.
- **Hover, scroll and motion** (before the rebase): 4000 pointer motions
  through scoot's IPC in 0.3 s (39 bar wakeups): 2.67, 2.61 and 2.21 ms on
  `3211551bd` with workspaces placed (which took a pointer), 2.55, 2.47 and
  2.31 ms with this change and the same flags, **0 ns and 0 wakeups for a
  clock-only bar with no binding** (it takes no pointer), and 2.51, 2.87 and
  2.70 ms with a binding on the clock (every entry and exit is a one-span
  repaint). The first hover with a binding allocates the second `wl_shm`
  buffer: RSS 3.6 MiB against 3.3.
- **What is left for the maintainer**: a ruling on a 5% larger binary and
  0.2 MiB more resident for the pointer (rule 1 reads "no row regresses";
  this one does), whether to shrink it (the `toml` fields are the largest
  piece), or to gate the interactions behind a feature, and the run on real
  hardware that decides it.

## Evidence

All on the dev VM (`ssh -p 2222 dev@localhost`, aarch64 NixOS, 6 vCPUs),
each tree in its own directory with its own `CARGO_TARGET_DIR`, shipped by
`tar` over `ssh` (the 9p mount was not used). The integration tests ran
against the VM's `scoot` built 2026-09-28 (ipc protocol 4), not `main`'s.
The checks below ran at `c9361fc4c` (this PR rebased onto `main` at
`e6ca6e35a`; the code is the same as the `37c52f7ad` they first ran at, and
the rebase's one real merge, `daemon/binds.rs`, which #363 added and this
extends to bind the seat for any module that takes pointer input, is in
it). The backlog-only commit after it changes no code.

```text
cargo fmt --check -p scootbar                                          ok
cargo clippy -p scootbar [FLAGS] --all-targets -- -D warnings          clean for FLAGS in:
  (default) | --no-default-features | --no-default-features --features clock
  | --no-default-features --features workspaces | --no-default-features --features icon-image
  | --features icon-image | --all-features
SCOOTBAR_TEST_SCOOT=/var/cargo-target/debug/scoot SCOOTBAR_REQUIRE_SCOOT=1 \
  cargo nextest run -p scootbar --no-fail-fast        Summary 533 tests run: 533 passed, 0 skipped
cargo nextest run -p scootbar --no-fail-fast  (target dir with no scoot)   533 passed (the integration tests skip)
SCOOTBAR_TEST_SCOOT=... SCOOTBAR_REQUIRE_SCOOT=1 cargo test -p scootbar    466 + 3 + 11 + 8 + 7 + 4 + 2 + 1 + 4 + 11 passed
cargo nextest run -p scootbar --bin scootbar --all-features                485 passed
  ... --no-default-features 341 | --features clock 413 | --features workspaces 400 | --features icon-image 359
nix build path:.#packages.aarch64-linux.scootbar                          ok (rebased tree)
nix build path:.#checks.aarch64-linux.scootbar-modules                     ok (rebased tree; asserts KillMode=process)
```

Negative controls, run and reverted: a release that fires on any release
fails `a_press_off_the_module_or_a_release_after_leaving_is_no_click`
(`["click", "click"]` for `["click"]`); clearing the pointer's focus on a
reload fails `a_reload_leaves_the_pointer_where_it_was` (a bug the first
draft had: after any `msg reload` the pointer was dead until it left the bar
and came back).

Not verified: touch (scoot cannot inject it; the bar never asks for a
`wl_touch`, pinned by `touch_and_the_keyboard_never_get_a_pointer`), real
hardware (any `--tty` or Asahi run: none was done), sway, and a restart of
the Nix user unit under a real `systemd --user` through
`scripts/scootbar-unit-test.sh` (only the transient-unit control above).

### After review (2026-10-01)

Four findings, fixed in `7b2c6a57c` (the code; this entry is the commit
after it and changes none). The checks above ran at the tree before it, and
are stale for what it touched, so they were re-run at it (below); the stack
was rebased onto `main` at `7a1f9030a` in between, which changes no file
under `crates/` (`git diff ba4ceef99 7b2c6a57c -- crates Cargo.lock` is
empty, `ba4ceef99` being the commit as it was tested).

- **A flaky test, found and fixed.** `a_launched_command_holds_none_of_the_bars_descriptors`
  failed once under load in review. The cause was in the test, not the bar:
  it compared the bar's descriptor count at a fixed moment after the command
  printed its last line, but the bar holds the child's pidfd until its loop
  reaps it, a turn later. The test now waits for the count to come back (up
  to the suite's 20 s patience); a leak is a count that never does. On the
  dev VM, the test binary run alone in a loop with six busy loops of CPU
  load (`~/m4/stress.sh`: one `--exact` run per iteration):

  ```text
  before (87bc33c02's test):  25 failures in the first 66 iterations
    (the loop was stopped after iteration 66; "left: 9 right: 8", once "left: 10")
  after  (7b2c6a57c):         0 failures / 100 iterations, load average 6.9
  ```
- **A third button after a cancelled chord armed Middle** while one of the
  first two was still held. The pointer now keeps the set of buttons down and
  arms a press only when none was, so a chord ends at the last release
  (`a_third_button_after_a_chord_arms_nothing_while_one_is_held`).
- Two comments were wrong: `spawn.rs` cited `tests/exec.rs` for the
  descriptor test (it is `tests/pointer.rs`), and `action.rs` said a `scoot`
  Cargo feature gates `{ scoot = "quit" }` (decision 1: there is none).

```text
checks at 7b2c6a57c's crates tree (shipped as ba4ceef99), dev VM, own targets in /dev/shm,
scoot built from this tree (/dev/shm/m4t/debug/scoot):
cargo fmt --check -p scootbar                                          ok
cargo clippy -p scootbar [FLAGS] --all-targets -- -D warnings          clean for FLAGS in:
  (default) | --no-default-features | ... --features clock | ... --features workspaces
  | ... --features icon-image | --features icon-image | --all-features
SCOOTBAR_TEST_SCOOT=/dev/shm/m4t/debug/scoot SCOOTBAR_REQUIRE_SCOOT=1 \
  cargo nextest run -p scootbar --no-fail-fast        Summary 534 tests run: 534 passed, 0 skipped
cargo nextest run -p scootbar --no-fail-fast  (target dir with no scoot)   534 passed (the integration tests skip)
SCOOTBAR_TEST_SCOOT=... SCOOTBAR_REQUIRE_SCOOT=1 cargo test -p scootbar    467 + 3 + 11 + 8 + 7 + 4 + 2 + 1 + 4 + 11 + 7 + 9 passed
cargo nextest run -p scootbar --bin scootbar --all-features                486 passed
  ... --no-default-features 342 | --features clock 414 | --features workspaces 401 | --features icon-image 360
fuzz crate: cargo check --locked --bins                                ok
```
