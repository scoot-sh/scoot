---
title: "XWayland: drops onto X windows do not land (pointer focus needs an X arm)"
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# XWayland: give the pointer focus an X arm, so drops onto X windows land

Filed 2026-09-25 by XWayland Phase 4, PR #246 (see
[`xwayland-support.md`](./xwayland-support.md)'s Phase 4 record). Serves
daily use: dragging a file into an X file manager, text into an X editor, or
a tab within an X app are ordinary actions an X user hits before any
X-to-Wayland drag.

## What does not work, measured

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

## Why

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

## What to do

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

## PROGRESS — the X arm landed; Wayland → X drops land; X → X waits on a fork flush

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

**X → X and in-window moves do not, yet, and that is deliberate.** Found
while building it: Smithay's `DndFocus for X11Surface` unmaps the XWM's
full-screen XDND proxy when an X drag enters an X window and maps it back
on leave -- but the remap (`xwm/dnd.rs` `leave`, offer-less branch) is
never flushed, at the pinned fork rev and upstream `928d4a9` alike.
Measured: an X drag that crossed an X window found *no window* under the
pointer over a Wayland window; one unrelated XWM request (whose handler
flushes) later, the proxy was there. Every X drag crosses an X window --
it starts over its own -- so turning X-origin drags on as-is would break
X → Wayland drops, which work today. So the X target is reached only
through an X offer, which only a drag from Wayland gets: its offer-less
branches (the remap, finishing an X drag) are unreachable, and X-origin
drags take exactly the `WlSurface` path they took before
(`an_x_drag_crossing_an_x_window_still_finds_the_proxy_over_wayland` pins
it; with the gate mutated away it fails "no window").

**What is left** (one short PR):

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

**Design notes.**

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

**Benchmarks** (release, this web container, 4 cores; each binary built
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

**Live matrix** (`mousepad` 0.7.0, one over `GDK_BACKEND=x11` and one
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
