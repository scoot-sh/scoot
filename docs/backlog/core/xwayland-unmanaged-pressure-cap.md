---
title: "No per-client XWayland unmanaged-window cap: override-redirect windows draw and hit-test without bound"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# No per-client XWayland unmanaged-window cap

Filed 2026-09-26 from the per-X-client managed-window cap
(`docs/backlog/core/xwayland-toplevel-cap.md`). That cap counts only managed
X windows, at `map_x11_window`: override-redirect windows enter through
`map_x11_unmanaged`, which neither claims nor releases.

Unmanaged windows never enter the core, so they carry none of the
arrangement cost the managed cap bounds. What scales with their count
instead:

- **The per-motion hit test** (`x11_unmanaged_under`) walks every
  override-redirect window's surface tree on every pointer motion.
- **The draw path and frame callbacks** reach every one of them per frame
  and output (`x11_unmanaged_frames`).

So one X client can still make every pointer motion and every frame cost
more, without bound and without ever touching the managed cap.

Low for the same reason as the managed cap: the X socket is the session's
own XWayland instance, not the open Wayland socket. The managed cap's shape
is the one to copy -- the same 128, the same client-bits identity, the same
refuse-the-map form (an override-redirect window that is refused is simply
never drawn) -- with its own counting, since unmanaged windows share no
path with the managed ones. Serves daily-drivability (a runaway X app's
menus should not make the pointer lag) more than computer use.

## Evidence expected

A harness test with the XWayland test rig (dev VM:
`SCOOT_REQUIRE_XWAYLAND=1`, X tools via `~/xw`): an X client mapping past
the cap has its menus refused while other X clients' menus still draw; the
count releases on unmap. No open-rate benchmark at the boundary is needed
(refusal, not perf) -- but no regression to normal menu mapping (existing
xwayland suites green).
