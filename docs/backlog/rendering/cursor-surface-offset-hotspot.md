---
title: "`wl_surface.offset` on a cursor surface doesn't move the hotspot (LOW, upstream gap)."
status: "open"
area: "rendering"
priority: "low"
blocked: null
---

# `wl_surface.offset` on a cursor surface doesn't move the hotspot (LOW, upstream gap).

**`wl_surface.offset` on a cursor surface doesn't move the hotspot (LOW,
  upstream gap).** Per `wayland.xml`, `hotspot_x`/`hotspot_y` should
  decrement on `wl_surface.offset` requests to a cursor surface. At the
  pinned Smithay rev, `CursorImageAttributes.hotspot` is only ever written
  by `wl_pointer.set_cursor` (and the tablet-tool equivalent) — nothing
  adjusts it on offset/commit — and flexwm reads it verbatim. A client
  using `wl_surface.offset` on its cursor gets a misplaced image. Not a
  regression (nothing rendered for `Surface` before item 8), and real
  toolkits don't appear to do this in practice.

**Security audit (2026-09-12, against `main` at `2b92928`).** A dedicated
security pass separate from the usual correctness/performance review found
one CRITICAL finding (any Wayland client can abort the whole compositor via
`wl_shm_pool.resize(0)`, a missing `return` in the pinned Smithay revision —
fixed as item 7 above, not listed here as backlog) and one HIGH finding
(the dev VM's forwarded SSH port bound to every interface instead of
loopback, exposing the documented hardcoded
credentials — and, since the shared `/mnt/flexwm` 9p mount has no read-only
option in this NixOS module, LAN write access to the actual host checkout —
to the whole LAN; fixed same-day, `host.address = "127.0.0.1"` added to
`vm/configuration.nix`, see its own commit and `vm/README.md`). The MEDIUM
and LOW findings below are real but lower-urgency; none were exploitable
data-loss/RCE in what was checked.
