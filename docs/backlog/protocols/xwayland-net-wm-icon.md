---
title: "XWayland: _NET_WM_ICON is not read"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# XWayland: `_NET_WM_ICON` is not read

Split out of [XWayland support](../resolved/xwayland-support-done.md) when
its Phases 5–7 closed it (2026-09-27).

## Today

An X window's icon is `_NET_WM_ICON`: raw ARGB pixels (one or more
`width, height, pixels...` CARDINAL runs), never a name. scoot does not read
it, and Smithay's `X11Surface` has no accessor for it at the pinned fork rev
(no `_NET_WM_ICON` atom anywhere in `smithay/src/xwayland/`).

## Why this is lower than it looks

scoot's only icon consumer is the `icon` field of `scoot msg windows`, and
that carries a **name** only: `xdg-toplevel-icon-v1` clients that send pixel
buffers instead of a name already read as having no icon
(`docs/protocols.md`, "Window icons": re-encoding buffers to PNG per query
was rejected), and neither foreign-toplevel protocol has an icon event. So
an X window reading as icon-less is exactly the parity a pixel-only xdg
client gets today. Bars resolve icons from the app id (`WM_CLASS` class for
X windows, e.g. `XTerm`) through `.desktop` files, which already works.

## If it is ever wanted

It only makes sense together with a pixel-icon path for xdg clients (an IPC
request returning one window's icon as PNG, on demand, through the
screenshot encoder's worker -- never per `windows` query). Then:

- **Files:** `compositor/xwayland/manage.rs` (read `_NET_WM_ICON` on map and
  on `PropertyNotify`, with a hard size cap -- a hostile client can publish
  a multi-megabyte property, and the Phase 2 hostile-property findings
  apply: bound the read, validate `width * height` against the run length
  with checked arithmetic), the IPC request and its docs.
- **Fork change:** none needed -- scoot can read the property over its own
  x11rb connection, the way `focus.rs` reads `_NET_STARTUP_ID`; an accessor
  in the fork would be nicer but is optional.
- **Risk:** low for scoot's stability if bounded; the cost is the IPC
  design, not the X side.

Recommendation: leave open and low, or close as won't-do alongside the
pixel-icon decision for xdg clients.
