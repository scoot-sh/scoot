---
title: "An override-redirect X window scoot refuses still costs the XWayland server its buffers"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-07"
---

# An override-redirect X window scoot refuses still costs the XWayland server its buffers

Filed 2026-09-27 from
[`xwayland-server-death-many-unmanaged`](../resolved/xwayland-server-death-many-unmanaged-done.md).
Serves **daily-drive**: a runaway X app should not be able to take down
every X app's windows on its own. It is low because the X socket is the
session's own, and because the XWayland server's budget now sits 8× above
the old death point.

## What happens

scoot's per-X-client caps (`toplevel_cap.rs`: 128 managed and 128
override-redirect windows per X client) refuse a map at the window manager.
The window is never drawn, hit-tested or put in the layout. For a
**managed** window that is the whole story: measured, it costs the X
server's Wayland connection nothing (below). For an **override-redirect**
window it is not. The X server allocates the window's pixmaps and commits
one to its `wl_surface` before scoot hears of the map, and each is a
`wl_shm` pool fd and a `wl_buffer` charged to the server's connection.
Measured: one X client mapping 240 override-redirect windows got 128 drawn,
yet the server held **480** buffers and 480 ledger fds, the same as if all
240 had been drawn.

So the override-redirect cap bounds what scoot draws and walks, but not
what the X server's connection costs. One X client can map menus until the
server reaches its budget (`xwayland_budget.rs`: 4096 on the usual table,
about 2048 windows). At that point scoot disconnects the server, and every
X client's windows go with it.

## Withholding `_XWAYLAND_ALLOW_COMMITS` does not fix it (measured)

This entry first proposed having the window manager withhold
`_XWAYLAND_ALLOW_COMMITS` from a refused window. The fork method for it
was written and published (scoot-sh/smithay `fcf6f314`,
`X11Surface::set_commits_allowed`), and measured in scoot before anything
was pinned. It does not help, and scoot does not pin it.

Headless harness, XWayland 24.1.13, one `x11rb` client mapping 120x90
windows filled by their background pixel. "Server" is the XWayland
server's connection: live `wl_buffer`s and fds in the ledger.

| Scene | Server buffers / fds |
| --- | --- |
| 200 managed windows mapped, 128 admitted, no change | 256 / 258 |
| 240 menus mapped, 128 drawn, no change | 480 / 480 |
| The same, with commits withheld at the refusal | 480 / 480 |
| 128 menus drawn, then 1 more mapped (refused), no change | 256 → 258 in one dispatch, 258 three seconds on |
| The same, with commits withheld at the refusal | 256 → 258, 258 three seconds on; 16 more mapped: 290 |
| 240 menus mapped, commits withheld at *creation* for every menu and allowed on admission | 480 / 480 |
| 240 menus *created* first, dispatched, then mapped, withheld at creation, allowed on admission | 368 / 368 (128 × 2 + 112 × 1) |

Why, read from the 24.1.13 source to explain what was measured:

- A refused window costs its two buffers before scoot hears of the map. The
  window's backing pixmap gets its pool and buffer when it is realized
  (`xwl_shm_create_pixmap`, `CREATE_PIXMAP_USAGE_BACKING_PIXMAP`). The
  first commit hands that pixmap to the compositor and allocates a second
  (`xwl_window_swap_pixmap` → `xwl_window_realloc_pixmap`). Both happen in
  the X server's dispatch of the client's own `MapWindow`, before the
  window manager has read the `MapNotify`.
- After that, nothing more is spent. A refused window is never sent a frame
  callback, and XWayland posts a window's next frame only once its last
  one's callback has fired (`xwl_screen_post_damage`). So withholding
  commits at the refusal stops commits that would not have happened
  anyway.
- XWayland reads the property once, when it realizes the window
  (`xwl_window_init_allow_commits`). A window manager that sets it at
  `CreateNotify` beats that only for a window created some time before it
  is mapped. A client that creates a window and maps it right away wins
  the race: its `MapWindow` is next in its own request stream, before the
  window manager has even read the `CreateNotify`. Even when the window
  manager wins, the backing pixmap's buffer is already spent, so the gain
  is one buffer in two. The cost falls on every menu scoot does draw: two
  property writes, and one window-manager round trip before its first
  frame.

## What would fix it

The window's buffers have to go, not just its further commits: XWayland
frees them when the window is unrealized (`xwl_window_dispose`). The
window manager could unmap or destroy a refused override-redirect window.
X lets any client do so, but Smithay's `X11Surface::set_mapped` refuses
override-redirect windows (`UnsupportedForOverrideRedirect`), and
`X11Surface::close` destroys a window only if it lacks `WM_DELETE_WINDOW`.
Either would need a fork method. Both are hostile to the app: an
override-redirect window is its own to map, a toolkit that finds its menu
destroyed under it can fail on its next request to it (`BadWindow`, which
kills a plain Xlib client), and a client that maps again at once turns
this into a loop. So it needs a design pass on how far past the cap to
tolerate first (only near the server's budget, say), not just a fork
commit.

## Evidence expected

- The live suite `compositor/xwayland/tests/refused_cost.rs` pins what
  holds today. A refused managed window costs the server nothing. A
  refused menu costs no more than a drawn one, and nothing more while it
  sits refused. Other X clients stay served.
- A fix adds a fail-first live test: one X client far past its
  override-redirect cap, and the server's buffer count stays near what its
  128 drawn menus cost.

## Resolved 2026-10-08 (PR #511)

Tolerate 64 refused menus past the 128 cap (192 mapped total), then kill
the runaway client's X connection (`XKillClient` over scoot's own X
connection, lazy, dropped with the server) -- so one client cannot spend
the server's budget on its own, while other X clients stay served. The
ticket's fork premise (unmap or destroy the refused window via a fork
method) was not needed and would not have kept up anyway: withholding
commits was already measured as no help, and closing refused windows one
by one was measured here and still disconnects (a paced 5000-menu storm
with thousands closed, Broken pipe; the frees lag the maps). Killing
stops the source; a later request by the dead client fails on its broken
connection, never as a dangling `BadWindow` in a live client. No fork,
no new dependency (Smithay re-exported `x11rb`), no hot-path cost for
legitimate maps.

Evidence (Asahi M2, XWayland 24.1.13, loadavg beside each number):
- Without the kill: 5000-menu storm FAILs, runaway mapped 4112 menus
  before its connection broke (server disconnected past its 4096
  budget), Broken pipe.
- With it: 21 tests pass (hermetic tolerance discipline + pins + storm
  in ~1.4 s, runaway killed near 193 maps, server under budget, another
  X client served).
- Measured live here, one mapped menu holds 1 buffer and 1 fd once
  settled (a 2500-storm holds 2500/2500); the ticket's 2-each was for a
  longer-settled scene with second frames.
- `cargo nextest run --workspace`, `cargo test -p scoot` (soft-egl),
  `cargo clippy` (default + `gpu-scanout`), `cargo fmt --check`,
  `scripts/smoke-test.sh`, `cargo deny check`, `scripts/backlog check`
  (3 pre-existing only), site build: all green (see PR #511 checks).
