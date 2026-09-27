---
title: "XWayland: an X drag released on its first motion into an X window may drop on the proxy"
status: "open"
area: "protocols"
priority: "low"
blocked: "a scoot-sh/smithay change (proposed below, verified locally, not pushed)"
---

# XWayland: an X drag released on its first motion into an X window may drop on the proxy

Filed 2026-09-27 from the final review of the pointer focus's X arm
([`xwayland-pointer-focus-x11-done.md`](../resolved/xwayland-pointer-focus-x11-done.md)).
Serves daily use (a quick flick-and-release drag between X apps) and
computer use (an agent's drag is a few `pointer move` jumps).

**Reproduced 2026-09-27, live with GTK and in the harness -- by two
mechanisms, one of them not the one this entry was filed for.** A fix is
worked out and verified against a patched local copy of the fork; it is a
scoot-sh/smithay change, not pushed (the session that found it had no
mandate to push to the fork), plus a one-method forward in scoot once the
fork is repinned. Priority stays low: every failing drag below needs the
release to come with no further motion after the pointer lands, which a
hand on a mouse rarely does and an agent often does.

## As filed (reasoned from code)

While an X app drags, the window manager's full-screen XDND proxy sits
over everything, so the X source finds it under the pointer and relays the
drag to Wayland through it. When scoot's drag grab moves onto an X window,
Smithay's `DndFocus::enter` for `X11Surface` unmaps the proxy and flushes,
so the source finds the real window. Nothing orders that unmap against the
pointer event the source reacts to, so on the drag's first motion into an
X window the source may still find the proxy, and a release on that motion
drops on it -- nothing lands.

## What was measured

Web container, 4 cores, Xwayland 24.1.13 (the flake's nixpkgs), scoot
`bca612b` (`main`), scoot-sh/smithay `d3a4cd73`.

### Live: GTK `mousepad` 0.7.0

`scoot --headless --width 2400 --height 600 --xwayland` (config
`[layout] default_column_width = 0`, three columns),
`RUST_LOG=info,smithay::xwayland::xwm=trace`; `mousepad --disable-server`
over `GDK_BACKEND=x11` (source "hello world", target "target") and, for the
Wayland cases, a native one between them. Driven over the IPC socket by a
script: fresh files and processes each drag, double-click "hello", press on
it, then per case, then **one** `pointer_move` onto the target's text and
the release -- `settled` (300 ms later) or `same` (move and release
pipelined on one connection, which scoot serves in one wakeup with one
flush, as a libinput batch). Landed = the target's title gained mousepad's
`*`. Proxy traffic = `Got XDND ... msg` in the log after the final move.

| case | `d3a4cd73` | with the patch below |
| --- | --- | --- |
| **direct**: 3 px steps only until GTK takes `XdndSelection`, then the jump to the other X window, settled | **0/20** landed; GTK spoke to the proxy after the jump in 20/20 | **20/20** |
| **own**: five 3 px steps in the source first, then the jump, settled | **18/20** (and 17/20 in a first run at 1600 wide); GTK spoke to the proxy in each failure | **20/20** |
| wl: own, three steps onto the Wayland window, then the jump, settled | 20/20 | 20/20 |
| wl, same step | 9/10, 9/10 | 8/10 |
| X to the Wayland window, one approach step then the jump, settled | 10/10 | 10/10 |
| X to the Wayland window, the jump alone, settled | 0/10 | not rerun |
| direct / own, same step | 0/10, 0/10 (0/20, 0/20 at 1600 wide) | 0/10, 0/10 |
| press, then the jump alone (GTK starts the drag on it), settled / same | 0/20, 0/10 | 0/20, 0/10 |

The last three rows are not this bug and do not move with the fix: GTK 3
decides a release on the target it had *before* the motion (its
`gtk_drag_button_release_cb` reads the context before the motion's
low-priority `gtk_drag_update_idle` has run), and nothing reached the
proxy. That would happen on any X server.

### Why "direct" fails: the proxy is never unmapped

The log of each failing direct drag: `New XDND selection`, then, after the
jump, no `XDND grab entered X11Surface` at all, and `Got XDND enter msg`
from GTK at the proxy. GTK (and Qt, from its code) looks for a target the
moment it takes `XdndSelection` -- before the window manager has made the
proxy -- so it finds its own window and names its types to nobody.
Smithay's `DnDGrab::update_focus` enters no target while the source has no
mime types ("delay until they have materialized"), so the jump enters
nothing and the proxy stays mapped over the target. GTK finds the proxy
there, names its types to it, is refused (no Wayland target chose an
action), and cancels on the release. Deterministic: the harness pin fails
3 of 3.

### Why "own" fails sometimes: the proxy flickers between two X windows

Here the grab has entered the source's window, so the jump leaves it --
`DndFocus::leave` maps the proxy back and raises it -- then enters the
target, which unmaps it. The X server sees map, raise, unmap. GTK looks
its targets up in a cache it keeps from root `SubstructureNotify` events
(`GdkWindowCache`, `get_client_window_at_coords`), and the raise's
`ConfigureNotify` is one of the events the X server flushes at once
("critical output", `dix/events.c`), so GTK can hold the proxy as mapped
and on top when it resolves, before the `UnmapNotify` arrives. In the
failing drags it spoke `XdndEnter` to the proxy after the jump.

### The harness: the filed ordering, alone, bites only an eager cache

`compositor/xwayland/tests/first_motion_race.rs` (ignored; ~15 minutes):
an x11rb source with the X server's implicit grab (its window selects
button and motion events), a second X client's `XdndAware` target, one
motion, the release. The source handles each event as it arrives and
picks its target two ways: `QueryPointer` down the tree when it handles
the motion (Qt, Chromium), or an eager map-state cache fed by root
`SubstructureNotify` (GTK's idea, minus GTK's deferral). Counts per case,
50 each:

Command (at scoot `bca612b` plus these tests, fork `d3a4cd73`):
`SCOOT_REQUIRE_XWAYLAND=1 FIRST_MOTION_N=50 cargo test -p scoot --features
xwayland --bin scoot first_motion_release_measurement -- --ignored
--nocapture --test-threads=1` (715 s). "Stale" = the source's own events
still had the proxy mapped when the motion came; "lost" = the source found
the proxy, dropped there, and the target got nothing.

| start | release | resolve | stale | found proxy / lost | target got the drop |
| --- | --- | --- | --- | --- | --- |
| announced (source named its types to the proxy) | same step | query | 1 | 0 | 50 |
| | | cache | 0 | 0 | 50 |
| | settled | query | 45 | 0 | 50 |
| | | cache | 45 | **45** | 5 |
| unannounced (as GTK and Qt start) | same step | query | 9 | 0 | 50 |
| | | cache | 3 | 3 | 47 |
| | settled | query | 50 | **50** | **0** |
| | | cache | 50 | **50** | **0** |
| via its own window | same step | query | 0 | 0 | 50 |
| | | cache | 0 | 0 | 50 |
| | settled | query | 0 | 0 | 50 |
| | | cache | 0 | 0 | 50 |

In every case scoot's grab had ended after the release and the next drag
started. In "unannounced, same step" the proxy window outlived the drag
(50/50, unmapped): the grab never entered anything, so nothing took the
window manager's `active_drag`; the next X drag replaces it ("Dropping
stale Xwm drag"), so it is not a wedge.

The "unannounced, settled" rows are the direct case above, for any
source: the proxy was never unmapped. (An earlier run of this file's
first draft, announced and via-own-window only, gave the same shape:
47/50 and 42/50 stale in the settled order, 42/50 lost to the cache, 0
to the query.)

- scoot flushes the unmap before it flushes the motion, but **the X server
  does not keep that order**: it handles input read in the same wakeup
  before client requests, and flushes a core `MotionNotify` at once, so in
  the settled order the source was told of the motion before the proxy's
  `UnmapNotify` in most drags.
- Where the grab did unmap the proxy (announced, via its own window), a
  source that **asks the server** found the target every time: its query
  can only arrive after it saw the motion, by which time the window
  manager's request, read earlier, has been processed.
- The eager cache found the proxy whenever the motion came first, and that
  drop was lost. GTK is not that eager -- it resolves in an idle after its
  pending events, and its XI2 motion is not flushed early -- which is why
  the live `direct` and `own` failures above have their own causes rather
  than this one.

What the harness cannot represent: the source reacts in the test's thread
between compositor dispatches; XWayland shares the test's four cores; and
the in-process source is not GTK. The live runs are what speak for GTK.

Found in passing, not investigated: in the harness, with the pointer
moving from one X window onto another and pressing in the same batch
(`pointer_move` then `pointer_button` with no dispatch between), XWayland
delivered the press to no X window. The measurement moves first and
settles, as a real pointer would.

## The fix (scoot-sh/smithay, then a scoot forward)

A new `DndFocus` method with a default, `enter_needs_metadata(&self, data,
source) -> bool` (default `true`), which `DnDGrab::update_focus` asks
before it delays entering a target for want of types, and which also lets
it enter such a target **before** leaving the old one:

- `X11Surface` answers `false` for the window manager's own drag onto
  another client's X window, and onto the source's own windows once the
  source has named its types. Entering an X window offers it nothing; the
  X source speaks to it itself. The source's own windows still wait while
  it has named nothing, so the proxy stays over them until the source
  names its types -- which a later Wayland target needs (without that
  exception, X-to-Wayland drops needed two more motions over the Wayland
  window: 0/10 at one and two approach steps, measured, where `d3a4cd73`
  lands 10/10 and 9/10).
- Entered first, `X11Surface::enter` records the hovered X window in the
  drag's state, and `leave` does not map the proxy back when a different X
  window has been entered since: no map/unmap between two X windows.

Verified with `cargo nextest run --config
'patch."https://github.com/scoot-sh/smithay".smithay.path="<patched copy>"'`
(own `CARGO_TARGET_DIR`): the three pins in
`compositor/xwayland/tests/first_motion.rs` pass 3 of 3 and fail 3 of 3
at `d3a4cd73`; the whole `--features xwayland` suite, 1869 passed, 25
skipped; and the live table's right column.

To land it: commit the patch to the fork, repin (`crates/scoot/Cargo.toml`,
`Cargo.lock`, `flake.nix`, `docs/forks.md`), forward the method in
`pointer_focus.rs` (both arms, to the `X11Surface` and the `WlSurface`),
and un-ignore the three pins.
