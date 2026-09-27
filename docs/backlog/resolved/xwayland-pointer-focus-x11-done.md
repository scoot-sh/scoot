---
title: "XWayland: drops onto X windows do not land (pointer focus needs an X arm) — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# XWayland: give the pointer focus an X arm, so drops onto X windows land — RESOLVED

RESOLVED 2026-09-27 (branch `claude/scoot-backlog-issues-3rfkfv`).
Drag-and-drop between X and Wayland apps works in every direction: X to
Wayland (as before), Wayland to X, X to another X app, and within one X
app. The pointer focus got its X arm (`compositor/pointer_focus.rs`; the
three commits under PROGRESS below), and the last piece -- X-origin drags
-- went through once scoot-sh/smithay `6e6fe896` flushed the XDND proxy's
remap (`X11Surface`'s `DndFocus::leave`, `let _ = xwm.conn.flush();` after
the `configure_window`, as `enter` does after its unmap). The fork is
repinned there (`crates/scoot/Cargo.toml`, `Cargo.lock`, `flake.nix`,
[`docs/forks.md`](../../forks.md)), and after review to `9515d7e5`, then
`d3a4cd73` (see the two review sections below).

- **The gate is gone.** An X focus now goes to Smithay's X target with an
  X offer or none (`PointerOffer::as_x11`); only a surface offer on an X
  focus -- which cannot happen -- goes the surface's way. Over an X window
  an X drag's target unmaps the proxy (the source finds the real window),
  maps it back on leave, and its `drop` ends the window manager's side of
  the X drag.
- **Tests.** The four ignored X-origin tests in
  `compositor/xwayland/tests/drop.rs` run: X to X
  (`an_x_drag_drops_onto_another_x_clients_window`), within one window
  (`an_x_drag_finds_its_own_window_under_the_pointer`), the hovered
  window closing (`an_x_drag_whose_hovered_window_closes_finds_the_proxy_again`),
  and the proxy back over Wayland, which duplicated the old gate-pinning
  test and was folded into it:
  `an_x_drag_crossing_an_x_window_still_finds_the_proxy_over_wayland` now
  asserts the proxy is out of the way over the X window *and* back over
  the Wayland one, which pins the fork flush.
- **Fail-first, measured** (web container, Xwayland 24.1.13 from the
  flake's nixpkgs, `SCOOT_REQUIRE_XWAYLAND=1`, `cargo nextest run -p scoot
  --features xwayland -E 'test(/xwayland::tests::(drop|dnd)::/)'`, three
  runs each): at scoot `a1f364c` with only the fork rev swapped back to
  `74edbf32`, 10 of 12 pass and the two proxy tests fail every run
  (`left: "no window"`, `right: "Smithay XDND proxy"`); at `6e6fe896`,
  12 of 12, every run. With the gate still in place (the merge commit
  `da869d4`, `--run-ignored all`), the four X-origin tests fail.
- **Benchmark:** not re-run. The change deletes a branch in `DndFocus`
  (drag-only paths: `enter` on each focus change of a drag, and
  `motion`/`leave`/`drop`); pointer motion without a drag, which
  `pointer_motion` and `x11_hot_path_cost` measure, does not reach it.
- **Live matrix, at `a1f364c`** (before the review fixes; web container, scoot `a1f364c` built `--features
  xwayland`, `scoot --headless --width 2400 --xwayland`, `mousepad` 0.7.0
  twice over `GDK_BACKEND=x11` and once native, one-third columns, driven
  by `scootctl pointer`/`screenshot`): X to X (a line from one X mousepad
  into the other; the source kept it), within one X window (a word moved
  to the end of its line), Wayland to X, and X to Wayland from a drag that
  started over its own X window -- the case the flush is for -- all land.
  One X-to-Wayland attempt did not: the scripted press came inside GTK's
  multi-click interval and extended the text selection instead of
  dragging (its screenshot shows the selection growing); with the pause
  before the press at 1.2 s it dropped. No `refusing an X drag` lines in
  the log. (Screenshots were in the session's scratch space.)

## Review follow-up -- resolved

Independent review of the X arm found two issues in Smithay's X drop target
that the Wayland-to-X half made reachable (`main` never made an X offer):

- **B1, blocking:** a Wayland drop onto an X target that never answers and
  then dies, or that is dropped on and then dies or hangs without
  `XdndFinished`, leaves the window manager's `active_offer` set, and it
  takes `XdndSelection` back from every later X drag until scoot restarts.
- **N1:** a validated drop tells the Wayland source `dnd_drop_performed`
  twice; a refused one is still sent `XdndDrop` (then `cancelled`).

Both fixed in the fork (pinned then at `9515d7e5`): `7388af13` tells the
source once and only for a drop made (a refused one is left with
`XdndLeave`), and `9515d7e5` ends the offer when its target or proxy is
destroyed, or when an X client takes `XdndSelection` after the drop --
narrowed by `d3a4cd73` below to an X drag `allow_drag` accepts. The
acceptance tests are `compositor/xwayland/tests/drop_end.rs` (six: never
answers, never answers then dies, refuses, dies after the drop, hangs
after the drop, and the finished control): all six fail at `6e6fe896`,
4 of 6 pass with `7388af13` alone, all six pass at `9515d7e5`. Review
note N3 (the drag gate's liveness check) is fixed on the scoot side;
N4 (the protocols.md wording) too.

## Final review -- no blocking issues

The final review, at `1ad66c1` (fork `9515d7e5`), found no blocking
issues. Its live spot-check at that head -- Wayland to X, X to Wayland, and
within one X window -- all landed, each with `Sending XdndDrop` then `Got
XDND finished msg` in the log. Its non-blocking notes:

- **N-A, fixed (fork `d3a4cd73`).** `9515d7e5` gave up a dropped but
  unfinished offer whenever *any* X client took `XdndSelection`, with
  nothing held, so a rough client could end a drop in flight and keep the
  selection. Now only a drag `allow_drag` accepts gives it up; any other
  taker is taken back from (flushed). Test:
  `drop_end.rs`'s `a_rough_x_client_cannot_end_a_pending_drop` -- fails at
  `9515d7e5` (`left: 8388608 right: 8388608`: the rough client kept the
  selection), passes at `d3a4cd73`; the drop, drop_end, dnd, xdnd,
  clipboard and peer suites 46/46, three runs; full `--features xwayland`
  nextest 1851 passed, 24 skipped.
- **N-B, fixed (same commit).** A new Wayland drag onto X that replaced a
  stale dropped offer never told that offer's source how its drop ended;
  it is now cancelled.
- **N-C, left.** A Wayland drag entering an X window makes synchronous
  property round trips to the X server (`XdndProxy`, `XdndAware`) on each
  X window it enters. Drag-only, once per enter; not measured as a cost.
- **First-motion race, left, unverified then.** Reasoned from the code:
  an X-origin drag released on its very first motion into an X window may
  send `XdndDrop` to the proxy scoot has just unmapped. Filed as
  [`xwayland-x-drag-first-motion-race-done.md`](./xwayland-x-drag-first-motion-race-done.md),
  since reproduced live (by a related mechanism: the proxy never unmapped
  at all) and resolved by fork `b1ac3ca7`.
- **Slow target, inherent.** If a genuinely new X drag starts while a
  slow target is still converting the previous drop, the target reads the
  new drag's data: X has one `XdndSelection`, and the new drag owns it.
  Nothing on the window manager's side can hold two.

## History: the entry as filed and as worked

Everything below is the record as it was written while the work was in
progress, kept for its measurements. Its present tense describes the tree
at the time -- it is superseded by the sections above: every direction
now lands, the X-origin gate is gone, and the fork fixes it describes as
pending are pinned.

Filed 2026-09-25 by XWayland Phase 4, PR #246 (see
[`xwayland-support.md`](../protocols/xwayland-support.md)'s Phase 4 record). Serves
daily use: dragging a file into an X file manager, text into an X editor, or
a tab within an X app are ordinary actions an X user hits before any
X-to-Wayland drag.

### What did not work, measured (before `a641f1c`)

Live on the dev VM (`~/evidence/xw4/live-dnd/`, `mousepad` 0.7.0 over
`GDK_BACKEND=x11` and native Wayland, scoot `--headless --xwayland`):

- X app → Wayland app: **works** (`01-x-to-wayland-after-drop.png`).
- X app → another X app: the drop does nothing (`02-x-to-x-after-drop.png`).
- Wayland app → X app: the drop does nothing (`04-wayland-to-x-after-drop.png`).
- Within one X app (moving selected text): the drop does nothing
  (`05-move-same-window-after-drop.png`).

No harm beyond the missing drop: the drag ends, the source text is intact
(checked after a same-window move, a move onto the other X app and a
release over the background -- `05`..`07`), and the X apps keep taking
input (`03-x-usable-after.png`: typed into an X window after the failed
drop).

This predates Phase 4. Phase 4's drag gate only ever refuses a drag;
an allowed X drag takes exactly the path `main` took before it (the window
manager's drag grab over a `WlSurface` focus).

### Why

Smithay's `DnDGrab::new_pointer` pins the drag's target type to
`SeatHandler::PointerFocus` (`input/dnd/grab.rs` at the pinned fork), and
scoot's is a plain `WlSurface` (`handlers.rs`). XWayland 24.1.13 binds no
`wl_data_device` at all (`strings` on the binary lists no data-device
interface), so an offer to an X window's `wl_surface` goes nowhere. X
windows take drops only through the window manager's XDND side, which
Smithay implements as `DndFocus for X11Surface` (`xwm/dnd.rs`) -- reachable
only when the grab's focus type can *be* an `X11Surface`. That is also what
unmaps the window manager's full-screen XDND proxy when an X-origin drag
enters an X window; with a `WlSurface` focus the proxy stays up for the
whole drag and catches the X source's own XDND traffic, which is why X → X
fails too, not only Wayland → X.

### What to do (the plan, since done)

Give the pointer focus an X arm, the way `SeatHandler::KeyboardFocus` got
one in Phase 2+3 (`keyboard_focus.rs`): an enum with
`Surface(WlSurface)` and, under the feature, `X11 { window, surface }`,
implementing `PointerTarget`, `WaylandFocus`, `IsAlive` and `DndFocus` by
delegating to the `WlSurface` or `X11Surface` impls. `surface_under` and
its callers (`input.rs`, `tablet.rs`, `relative_pointer.rs`,
`floating/grab.rs`, the popup grab's `From<KeyboardFocus>` conversion,
`xwayland/dnd.rs`'s gate) produce and read it.

- **Land it alone first, behaviour-neutral** -- the `KeyboardFocus`
  precedent -- then turn on the X arm.
- **It is on the pointer-motion hot path.** Benchmark before and after
  (`headless/bench/pointer.rs`, `cargo test -p scoot --bin scoot
  pointer_motion -- --ignored --nocapture --test-threads=1`, and the
  `--tty` jiffies sample): the enum
  clone is reference-count bumps, like `KeyboardFocus`'s, but measure it.
- Check `TabletSeatHandler::ToolFocus` and `TouchFocus` (which match
  `PointerFocus` today for the same bound) and the pointer-constraint and
  relative-pointer paths, which want the `wl_surface`.
- Tests: an X → X drop and a Wayland → X drop landing, on `Harness` with the
  x11rb client speaking the XDND target side, plus the live `mousepad`
  matrix above.

### PROGRESS at `7366568` — the X arm landed; Wayland → X drops land; X → X waited on a fork flush

Three commits on one branch (2026-09-27), the `KeyboardFocus` precedent
plus a measured optimization:

1. **`a641f1c`, behaviour-neutral.** `SeatHandler::PointerFocus` became
   `pointer_focus::PointerFocus`, one variant (`Surface(WlSurface)`)
   forwarding every `PointerTarget`, `WaylandFocus`, `IsAlive` and
   `DndFocus` call to the surface's own impl. `surface_under` answers it;
   the constraint paths (`absolute_target`, `engage_pending_constraint`,
   `new_constraint`, `deactivate_pointer_constraint`) look up and compare
   by the focus's *surface*, a constraint being made on a `wl_surface`.
   `TabletSeatHandler::ToolFocus` and `TouchFocus` stay `WlSurface` (a
   tool never drags; touch drags are refused both sides) --
   `State::tool_under` unwraps the focus for the tool. The popup grab
   converts `KeyboardFocus` → `PointerFocus`.
2. **`1e642f6`, the X arm.** `X11 { window, surface }` in an `xwayland`
   build, produced by `window_under` (`PointerFocus::on_window`) and
   `x11_unmanaged_under` -- every X window, managed or override-redirect.
   Pointer events still go to the surface. The drag gate in
   `xwayland/dnd.rs` reads the pressed window off the focus instead of
   walking every X window for the surface.
3. **`7366568`, the X arm shared.** Holding the `X11Surface` by value (as
   `KeyboardFocus` does) measured +0.6-0.7 us per motion over X windows
   in release; the arm now holds an `Arc<X11Surface>` made once per window
   (cached in the managed window's Smithay `Window` user data; kept as the
   element of `State::x11_unmanaged` for override-redirect windows).

**Wayland → X lands**, bytes and all (`xwayland/tests/drop.rs`,
`a_wayland_drag_drops_onto_an_x_window`: x11rb target gets `XdndEnter`
with the Wayland type, `XdndPosition` at the pointer, answers
`XdndStatus`, gets `XdndDrop`, converts `XdndSelection` and reads the
Wayland source's payload). Fail-first at `a641f1c`: "timed out waiting
for XdndEnter".

**At `7366568`, X → X and in-window moves did not land yet, deliberately**
(fixed since by `6e6fe896` and `a1f364c`). Found while building it:
Smithay's `DndFocus for X11Surface` unmaps the XWM's full-screen XDND
proxy when an X drag enters an X window and maps it back on leave -- but
the remap (`xwm/dnd.rs` `leave`, offer-less branch) was never flushed, at
the fork rev pinned then (`5b575329`) and upstream `928d4a9` alike.
Measured: an X drag that crossed an X window found *no window* under the
pointer over a Wayland window; one unrelated XWM request (whose handler
flushes) later, the proxy was there. Every X drag crosses an X window --
it starts over its own -- so turning X-origin drags on as-is would have
broken X → Wayland drops, which worked then. So at that commit the X
target was reached only through an X offer, which only a drag from
Wayland got: its offer-less branches (the remap, finishing an X drag)
were unreachable, and X-origin drags took exactly the `WlSurface` path
they took before (`an_x_drag_crossing_an_x_window_still_finds_the_proxy_over_wayland`
pinned it then; it now pins the flush instead, see the top section).

**What was left then** (all since done: the fork flush is `6e6fe896`,
the gate went in `a1f364c`, and the matrix was re-run at `a1f364c` --
see the top section):

- A scoot-sh/smithay commit on `5b575329`: `let _ = xwm.conn.flush();`
  after the remap's `configure_window` in `X11Surface`'s `DndFocus::leave`
  (like the unmap in `enter`). Written and verified locally -- with it
  patched in (`cargo --config 'patch."https://github.com/scoot-sh/smithay".smithay.path=…'`)
  and the gate removed, at `7366568`, all nine drop tests (the four
  ignored X-origin ones included) and the four drag-gate tests pass -- but
  it is not pushed: this session had no mandate to push to the fork. Pin
  it, add it to `docs/forks.md`.
- Drop the X-origin gate in `pointer_focus.rs` (`from_x` in `enter`, and
  let offer-less X foci reach the X target), and un-ignore the four
  X-origin tests in `drop.rs` (X → X, in-window, proxy back on leave,
  hovered window closing).
- The live `mousepad` matrix again.

**Design notes** (still current).

- `PartialEq` is written out: Smithay compares the new focus with the old
  on every motion, and `X11Surface`'s own eq takes both windows' state
  locks to fold liveness in. The X arm compares surface, window id and XWM
  id, lock-free (`motion_within_an_x_window_enters_it_once` pins it on the
  interaction ring: mutated to always-unequal, twenty motions filed twenty
  enters).
- The X arm carries its window, shared: an `X11Surface` is 432 bytes and
  about eight reference counts, and a motion clones and drops the focus
  several times (hit test, Smithay's stored copy, `current_focus`, the
  relative event), so by value it cost ~0.6-0.7 us per motion over X
  windows and made every focus 496 bytes to move. `Arc`, made once per
  window: `PointerFocus` is 72 bytes (64 without the feature). It carries
  the window rather than looking it up by surface as the drag moves
  because a closed window is not there to look up, and a Wayland drag's
  offer left on a closed X window makes the XWM take `XdndSelection` back
  from every later X drag
  (`a_wayland_drag_whose_x_window_closes_leaves_x_drags_working`).
  (`X11Surface`'s own user data cannot hold the `Arc`: it would be a
  reference cycle.)

**Benchmarks, at `7366568`** (release, this web container, 4 cores; each binary built
in its own worktree and target dir; base `87e1935` and head interleaved,
three rounds of the benches' own 5 x 20000 medians):

| | base | `1e642f6` (by value) | `7366568` (shared) |
| --- | --- | --- | --- |
| `x11_hot_path_cost`, motion over 3 X windows | 2.21 / 2.20 / 2.28 us | 2.97 / 2.91 / 2.79 us | 1.93 / 1.83 / 2.10 us (base 1.98 / 2.21 / 2.20 that run) |
| same, over 3 Wayland windows | 2.15 / 2.02 / 2.08 us | 2.13 / 2.21 / 2.43 us | 1.98 / 1.54 / 1.54 us (base 1.95 / 1.98 / 1.79) |
| `pointer_motion`, no drag, xwayland build | 1.40 / 1.63 / 1.47 us | 1.54 / 1.61 / 1.65 us | 1.41 / 1.34 / 1.57 us (base 1.49 / 1.32 / 1.54) |
| `pointer_motion`, no drag, default build | 1.51 / 1.49 / 1.48 us | 1.54 / 1.48 / 1.52 us | 1.15 / 1.48 / 1.49 us (base 1.49 / 1.50 / 1.46) |

The head is within noise of the base everywhere. The `--tty` jiffies
sample was **not** taken: this was built in a web container with no dev
VM and no `--tty` hardware.

**Live matrix, at `7366568`** (`mousepad` 0.7.0, one over `GDK_BACKEND=x11` and one
native, `scoot --headless --xwayland`, driven by `scoot msg`; same script
at base `87e1935` and head `7366568`):

- Wayland → X: base, the X window stays empty; head, the text lands.
- X → Wayland: works on both.
- Within one X window: nothing moves on either (the X-origin half), no
  text lost, the X app keeps typing afterwards.
- The XWM's `BadWindow` log lines around the drags are the same on both
  (5 `ChangeProperty`, 14 `GetProperty`, 4 `GetWindowAttributes`).

(Screenshots and logs were in the session's scratch space, which does not
outlive it; the script is reproducible from the description above.)
