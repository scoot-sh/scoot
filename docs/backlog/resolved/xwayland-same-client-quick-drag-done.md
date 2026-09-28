---
title: "XWayland: a quick drag between two windows of the same X app instance drops nothing"
status: "resolved"
area: "protocols"
priority: "low"
blocked: null
---

# XWayland: a quick drag between two windows of the same X app instance

Filed 2026-09-27 from the independent review of fork `b1ac3ca7`
([`xwayland-x-drag-first-motion-race-done.md`](../resolved/xwayland-x-drag-first-motion-race-done.md)).
Serves daily use: dragging text between two windows of one editor, or a
file between two windows of one file manager.

## Measured

Live, GTK `mousepad` over X, the second window opened with ctrl+shift+n in
the same process, scoot at `94008f4` (fork `b1ac3ca7`): a drag reaching the
other window in one motion and released there landed **0/5**; with five
motions over the source first, **4/4**. GTK sent `XdndEnter`/`XdndLeave` to
the window manager's proxy every time in the failing case. Not a
regression: before `b1ac3ca7` the same drag between two *different* X apps
failed the same way.

## Why

`b1ac3ca7` lets an X drag enter an X window without waiting for the
source's types -- except over the drag owner's own client, where it still
waits until the source has named them, so the proxy stays over the
source's window until then (the drag needs the types before it can enter a
Wayland window; letting the source's own windows skip the wait regressed X
to Wayland to 0/10). "The owner's client" is every window of the process:
single-instance apps (mousepad by default, GApplication apps generally)
run all their windows on one X connection, so their other window waits
too, the proxy stays over it on the first motion, and the source names its
types to the proxy.

## What to do

Wait only over the window the drag started on, not the whole client. The
`XdndSelection` owner cannot stand in for it (GTK's is a hidden IPC
window); the press that started the drag can -- the grab's start data
names its focus, which the window manager can map to an X window. Pin it
with a harness test like `first_motion.rs`'s own-client crossing test, but
released on the first motion, fail-first against `7e18b661`.

## Resolution (2026-09-27)

Fixed in scoot-sh/smithay `b16cd6a2`, as proposed above: `XwmActiveDrag`
records the X window the drag's press or touch started on -- the grab's
start focus, matched to an X window by its `wl_surface` -- and
`X11Surface::enter_needs_metadata` waits for the source's types only over
that window. When the start focus names no X window (it should always
name one for a drag the window manager accepted), every window of the
owner's client waits, as before.

**Fail-first.** `compositor/xwayland/tests/first_motion.rs`'s
`an_x_drags_first_motion_onto_its_own_clients_other_window_finds_that_window`
(a drag from one of the test client's windows onto its other window, one
motion) fails 3 of 3 at `7e18b661` ("found \"Smithay XDND proxy\" under
the pointer") and passes 3 of 3 at `b16cd6a2`.

**No regression.** The first_motion, drop, drop_end, dnd, xdnd, clipboard
and peer suites passed 51/51 three times against the fix (X to Wayland
drops, which start over the source's own window and still need its types,
included). The full set on the repinned tree is in the PR.

**Not re-verified live** with two mousepad windows of one instance; the
harness pin carries the mechanism.
