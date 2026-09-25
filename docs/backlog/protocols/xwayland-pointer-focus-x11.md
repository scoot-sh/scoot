---
title: "XWayland: drops onto X windows do not land (pointer focus needs an X arm)"
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# XWayland: give the pointer focus an X arm, so drops onto X windows land

Filed 2026-09-25 by XWayland Phase 4 (see
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
