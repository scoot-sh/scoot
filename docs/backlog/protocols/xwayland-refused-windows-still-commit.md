---
title: "An X window scoot refuses still costs the XWayland server its buffers"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# An X window scoot refuses still costs the XWayland server its buffers

Filed 2026-09-27 from
[`xwayland-server-death-many-unmanaged`](../resolved/xwayland-server-death-many-unmanaged-done.md).
Serves **daily-drive**: a runaway X app should not be able to take down
every X app's windows on its own. It is low because the X socket is the
session's own, and because the XWayland server's budget now sits 8× above
the old death point.

## What happens

scoot's per-X-client caps (`toplevel_cap.rs`: 128 managed and 128
override-redirect windows per X client) refuse a map at the window manager.
The window is never drawn, hit-tested or put in the layout. But the X
server does not know that. It still allocates the window's pixmaps and
commits them to its `wl_surface`, and each of those is a `wl_shm` pool fd
and a `wl_buffer` charged to the server's Wayland connection. Measured: one
X client mapping 240 override-redirect windows got 128 drawn, yet the
server held **480** buffers and 480 ledger fds, the same as if all 240 had
been drawn.

So the X-side caps bound what scoot draws and walks, but not what the X
server's connection costs. One X client can therefore map windows until the
server reaches its budget (`xwayland_budget.rs`: 4096 on the usual table,
about 2048 windows). At that point scoot disconnects the server, and every
X client's windows go with it.

## The fix this needs

XWayland stops committing a window's buffers while
`_XWAYLAND_ALLOW_COMMITS` is 0 (`xwl_window->allow_commits`, checked in the
damage-posting loop in `hw/xwayland/xwayland-screen.c`). Only the window
manager's X client may write that property (`xwl_access_property_callback`
refuses everyone else with `BadAccess`). In scoot that client is Smithay's
`X11Wm` connection, and the write is the private
`X11Surface::set_allow_commits`. Proposed fork commit on
`scoot/xwayland-selection-dnd` (against `b16cd6a2`,
`src/xwayland/xwm/surface.rs`):

```rust
    /// Tells the X server whether to commit this window's buffers to its
    /// `wl_surface` at all. A window manager that refuses a window can
    /// withhold them, so the refused window costs the compositor nothing.
    /// Smithay re-enables commits only when a sync request finishes, which
    /// never happens for a window that is never configured.
    pub fn set_commits_allowed(&self, allowed: bool) {
        let state = self.state.lock().unwrap();
        self.set_allow_commits(&state, allowed);
    }
```

scoot would then call `set_commits_allowed(false)` at both refusal sites
(`map_x11_unmanaged` and the managed map refusal). It would call
`set_commits_allowed(true)` if a refused window is ever admitted later,
which does not happen today, because a refused map stays refused until it
is mapped again.

## Evidence expected

- Measure first. XWayland may commit a window's first buffer before the
  window manager sees `MapNotify` and refuses it. If it does, the withheld
  window still holds one or two buffers, and the fix only halves the cost.
  The shape to reproduce is the one above: one X client, 240 menus, then
  compare the server's buffer count with 128 drawn.
- Then a live test: one X client far past its cap keeps the server's buffer
  count near what its drawn windows cost, and other X clients stay served.
