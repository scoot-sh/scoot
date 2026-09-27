---
title: "XWayland: an X drag released on its first motion into an X window may drop on the proxy"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# XWayland: an X drag released on its first motion into an X window may drop on the proxy

Filed 2026-09-27 from the final review of the pointer focus's X arm
([`xwayland-pointer-focus-x11-done.md`](../resolved/xwayland-pointer-focus-x11-done.md)).
Serves daily use (a quick flick-and-release drag between X apps).

**Reasoned from the code, not measured or reproduced.** Nothing here has
been seen to happen; it is a window the code leaves open.

## The suspected gap

While an X app drags, the window manager's full-screen XDND proxy sits
over everything, so the X source finds it under the pointer and relays the
drag to Wayland through it. When scoot's drag grab moves onto an X window,
Smithay's `DndFocus::enter` for `X11Surface` (scoot-sh/smithay
`xwm/dnd.rs`) unmaps the proxy and flushes, so the source finds the real
window under the pointer and speaks XDND to it directly.

Nothing orders that unmap against the pointer event the X source reacts
to: the motion reaches the source through XWayland, the unmap goes out on
the window manager's own X connection. So on the drag's very first motion
into an X window, the source may still resolve the proxy as the window
under the pointer. If the button is released on that same motion, the
source sends `XdndDrop` to the proxy, which is by then unmapped, and
scoot's grab ends over the X window with no offer (`DndFocus::drop` with
none marks the X drag finished). The likely result is a drop that does
nothing -- the drag ends, nothing lands -- rather than a wedge; that too
is unverified.

## What to do

1. Try to reproduce it first: an x11rb source (the `xwayland/tests`
   helpers) pressing on its own window, one motion into a second X
   client's `XdndAware` window, released at once; see which window gets
   `XdndPosition`/`XdndDrop`, and whether the target sees anything. Repeat
   enough times to call a race either way.
2. Only if it reproduces: decide where it belongs (likely the fork's
   `enter`/`drop` path, e.g. treating a drop that arrives at the unmapped
   proxy as a drop on the window under the pointer), measured fail-first
   like the fork's other XDND commits.
