---
title: "XWayland: _NET_WM_ICON is not read"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-07"
---

# XWayland: `_NET_WM_ICON` is not read

Split out of [XWayland support](xwayland-support-done.md) when
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

## Resolution — verify-first close (won't-do, parity stands)

The pixel-icon decision this ticket waited on has since landed:
[`toplevel-icon-buffers`](toplevel-icon-buffers-done.md) resolved
2026-09-18 as name-only stands, verified — pixels never leave the
compositor, and pixel exposure reopens only if a bar-over-IPC or an agent
visual need actually arrives (none has). Reading `_NET_WM_ICON` now would
hand scoot pixels it has already decided not to carry for xdg clients, so
this closes the same way: deliberately not read, not deferred.

Verified against the pinned fork rev `fdf424d`
(`~/.cargo/git/checkouts/smithay-*/fdf424d`, checked 2026-10-08):

- No `_NET_WM_ICON` atom anywhere in `smithay/src/xwayland/` (grep empty),
  no icon accessor on `X11Surface` (it exposes `title`, `class`,
  `instance`, `startup_id`, `pid`, `opacity`, … — no icon), and no icon
  variant on `WmWindowProperty` (`surface.rs:283`): an icon change would
  arrive as `Other(atom)`, which scoot's `property_notify` (`wm.rs:121`)
  ignores. The ticket's "no accessor" claim holds at the current rev.
- No fork change needed and none made: there is nothing to expose the
  pixels through (IPC `windows` `icon` is a name only; neither
  foreign-toplevel protocol has an icon event), so a scoot-side read would
  have no consumer. The hostile-property caution stands for any future
  revisit: bound the read, validate `width * height` against the run
  length with checked arithmetic.
- X windows already read as icon-less over IPC today, by construction:
  `State::icon_name_of` (`toplevel_icon.rs:140`) reads the xdg surface's
  cached state, and an X window has no toplevel, so it reports `None` —
  exactly the parity a pixel-only xdg client gets. Bars resolve X icons
  from the app id (`WM_CLASS` class via `x11_app_id`) through `.desktop`
  files, which already works.

Docs in this PR: the user-facing "Not yet" becomes a deliberate statement
(site `scoot/protocols.md`, XWayland section and Window icons section),
the IPC `icon` field documents that X windows always report `None`
(`scoot-ipc`), and `manage.rs` names the parity. No behavior change, no
test (docs-only close: nothing to fail first), no `PROTOCOL_VERSION` bump,
no CHANGELOG (nothing user-visible changed).

Revisit if and when the xdg pixel path reopens (a real consumer asking
for pixels over IPC): then read `_NET_WM_ICON` on map and on
`PropertyNotify` over scoot's own x11rb connection, bounded as above.
