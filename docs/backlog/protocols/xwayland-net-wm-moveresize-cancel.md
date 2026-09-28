---
title: "XWayland: honour _NET_WM_MOVERESIZE_CANCEL"
status: "open"
area: "protocols"
priority: "low"
blocked: "needs a scoot-sh/smithay fork hook: the XWM drops direction 11"
---

# XWayland: honour `_NET_WM_MOVERESIZE_CANCEL`

Split out of [`_NET_WM_MOVERESIZE`](../resolved/xwayland-net-wm-moveresize-done.md)
(2026-09-28). Serves daily use.

## Today

An X app's titlebar drag runs through scoot's floating grab until its
button is released. A client that wants to end it early sends
`_NET_WM_MOVERESIZE` with direction 11 (`_NET_WM_MOVERESIZE_CANCEL`), and
Smithay's window manager drops it (`xwm/mod.rs`, the `_NET_WM_MOVERESIZE`
arm: `_ => {} // ignore keyboard moves/resizes for now`), so scoot never
sees it. `xwayland/tests/moveresize.rs`,
`a_cancel_is_not_delivered_and_the_release_still_ends_the_drag`, pins that
the cancel is not delivered and that the release still ends the drag.

Nothing sticks without it. A request handled after its release is refused
(no click grab is held), and the release always reaches scoot's grab
first, so a client cannot see a release scoot has not (bar a request still
queued across a release and a new press in the same client, which rides the
new press and ends with its release). The cancel would only
let a client end a drag while the button is still held.

## Shape of the fix

A fork commit adding a default-empty `XwmHandler::move_resize_cancel(xwm,
window)` hook, called from direction 11 (9 and 10 stay dropped: scoot
refuses keyboard moves by design). Then, in `xwayland/moveresize.rs`: end
the floating grab (`State::end_floating_grab`, then `apply()`) only when it
is dragging that window, and only for a request from the window's own X
client -- which, as for the move itself, the window manager cannot tell
from a stranger naming the window. Flip the pinning test to "the cancel
ends the drag". List the commit in `docs/forks.md`.
