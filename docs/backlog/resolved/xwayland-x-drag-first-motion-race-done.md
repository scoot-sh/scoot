---
title: "XWayland: an X drag released on its first motion into an X window may drop on the proxy — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# XWayland: an X drag released on its first motion into an X window may drop on the proxy — RESOLVED

RESOLVED 2026-09-27 (branch `claude/scoot-backlog-issues-3rfkfv`), by
scoot-sh/smithay `b1ac3ca7` ("dnd: an X drag enters another client's X
window without waiting for types", on `d3a4cd73`) and a one-method forward
in `compositor/pointer_focus.rs`. Filed from the final review of the
pointer focus's X arm
([`xwayland-pointer-focus-x11-done.md`](./xwayland-pointer-focus-x11-done.md)).
Served daily use (a quick flick-and-release drag between X apps) and
computer use (an agent's drag is a few `pointer move` jumps).

## Resolution

**Verdict: it reproduced, by two mechanisms -- neither quite the one it
was filed for.** Details and raw tables in the record below.

- **Direct** (the drag's first motion onto another client's X window):
  GTK and Qt take `XdndSelection` and look for a target before the window
  manager's proxy exists, so they name their types to nobody; Smithay's
  `DnDGrab` entered no target without types, so the proxy was never
  unmapped and the drop went to it. Live, GTK `mousepad` over X: **0/20**
  landed; the harness pin failed 3 of 3.
- **Own** (a drag that crossed the source's own window first): leaving it
  mapped and raised the proxy, entering the target unmapped it, and the
  raise's `ConfigureNotify` could reach GTK's window cache first. Live:
  **18/20** (17/20 in a first run).
- The filed mechanism (the X server delivering the motion before the
  proxy's `UnmapNotify`) is real but, measured in
  `compositor/xwayland/tests/first_motion_race.rs`, bites only a source
  that resolves from an eager event cache at once on the motion (45/50
  lost, "announced, settled, cache"); a source that queries the server
  never lost one, and GTK defers its lookup to an idle.

**The fix** (fork `b1ac3ca7`, touching Smithay's generic `input/dnd` as
well as the XWM): a new defaulted `DndFocus::enter_needs_metadata(&self,
data, source) -> bool` (default `true`, today's behavior). `DnDGrab`
asks it before delaying an enter for want of types, and enters such a
target before leaving the old one. `X11Surface` answers `false` for an X
drag over another client's X window (or the source's own, once it has
named types), and its `leave` skips the proxy remap when another X window
was entered since -- no map/unmap between two X windows. scoot forwards
the method from `PointerFocus` (the X arm to the `X11Surface`, everything
else to the `WlSurface`, which keeps the default). Wayland-origin drags
and drags onto Wayland windows keep today's order and gate. Repinned in
`crates/scoot/Cargo.toml`, `Cargo.lock`, `flake.nix` and
[`docs/forks.md`](../../forks.md).

**Fail-first.** The three pins in
`compositor/xwayland/tests/first_motion.rs` (now un-ignored) fail on every
run at `d3a4cd73` (the first with "found \"Smithay XDND proxy\"", the
two crossing pins with "mapped the proxy back in between"); with only the
metadata half of the patch, the crossing pin still fails; with
`b1ac3ca7`, all three pass.

**Measured with the fix** (the coordinating session, against a local path
patch of exactly `b1ac3ca7`): the first_motion, drop, drop_end, dnd, xdnd,
clipboard and peer suites 49/49, three runs; full `--features xwayland`
nextest 1870 passed, 25 skipped; workspace 2357 passed, 25 skipped.
Live, mousepad over X: direct one-motion X to X drop **0/20 -> 20/20**;
five steps then the jump **18/20 -> 20/20**; X to Wayland unchanged,
10/10. Rerun on the repinned tree (the commit that moved this entry
here, fork from git): `--features xwayland` nextest with
`SCOOT_REQUIRE_XWAYLAND=1`, 1870 passed, 25 skipped; the same seven
suites 49/49, three runs; workspace nextest 2357 passed, 25 skipped;
clippy `-D warnings` clean with and without `--features xwayland`; `fmt
--check` clean; `scripts/smoke-test.sh` passed. The live GTK drags were
not rerun on the repinned tree (same fork source as the path patch).

**Not fixed, and not fixable here.** A direct or own drag whose release
comes in the same step as the motion onto the target fails on every build
(0/10 before and after): GTK 3 decides a release on the target it had
*before* the motion (`gtk_drag_button_release_cb` reads the context before
the motion's low-priority `gtk_drag_update_idle` runs), and nothing reaches
the proxy. That would happen on any X server; the compositor cannot
reorder a toolkit's idle. Documented for users and agents in
[`docs/protocols.md`](../../protocols.md#clipboard-drag-and-drop-and-input-methods):
let the move settle before the release.

**Unverified.**
- No live Qt or Chromium test; only GTK (`mousepad`) was driven live. Qt's
  start-of-drag behavior is from its code; the harness's `QueryPointer`
  source stands in for Qt and Chromium.
- "wl, same step" (an own drag crossing onto the Wayland window, released
  in the same step) went 9/10, 9/10 before to 8/10 with the fix. Small,
  within noise at ten drags, and unexplained; not chased.
- Found in passing: in the harness, a move from one X window onto another
  followed by a press in the same batch never delivered the press to any X
  window. Filed separately as
  [`xwayland-press-after-crossing-move-lost.md`](../protocols/xwayland-press-after-crossing-move-lost.md).

**Independent review of `b1ac3ca7` (branch at `94008f4`): no blocking
findings.** It re-ran the full set (workspace nextest 2443 passed;
`--features xwayland` 1884 passed; the drag suites 84/84 six times;
clippy, fmt and both smoke builds clean; the flake hash matched) and a
live spot-check with GTK `mousepad` over X at HEAD: one-motion X to X
**6/6**, X to Wayland **6/6**. Its findings:

- **A window mapped again under an X drag got the proxy over it** --
  fixed. The same window id under a new `wl_surface` is a new focus, so
  the grab entered it before leaving the old surface, whose `leave` saw its
  own window as the one hovered and mapped the proxy back. Fork `7e18b661`
  counts X windows entered and not yet left instead. Pinned by
  `first_motion.rs`'s `an_x_drag_over_a_window_that_remaps_keeps_the_proxy_away`
  (a managed window; an override-redirect one re-derives the pointer focus
  as it unmaps and never takes that order): fails 3 of 3 at `b1ac3ca7`,
  passes 3 of 3 at `7e18b661`. With it, the seven drag suites 50/50 three
  runs, `--features xwayland` nextest 1885 passed, 25 skipped.
- **A quick drag between two windows of the same X app instance still
  drops nothing** -- not fixed, not a regression: `enter_needs_metadata`
  waits over every window of the drag owner's client, and single-instance
  apps (mousepad by default, GApplication apps generally) run all their
  windows on one X connection. Live at HEAD: direct **0/5**, five steps
  first **4/4**. Filed as
  [`xwayland-same-client-quick-drag-done.md`](./xwayland-same-client-quick-drag-done.md) (fixed by fork `b16cd6a2`)
  and documented in `docs/protocols.md` and the CHANGELOG.
- `pointer_focus.rs`'s doc comment said `enter_needs_metadata` is asked
  only on a focus change; it is also asked on each motion while the drag
  has entered nothing yet -- corrected (the cost there is a lookup, no
  allocation, off the idle-pointer path; no benchmark warranted).
- `X11Surface::leave` logged a remap before checking whether it would
  remap -- fixed in `7e18b661`.
- The fork's own `anvil` does not forward `enter_needs_metadata`; it
  matters only if this is ever offered upstream.

## History: the entry as filed and as worked

Kept verbatim for its measurements. Its present tense ("not pushed",
"proposed below") describes the tree before the fork commit and repin.

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
