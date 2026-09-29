---
title: "A real maximize: fill the usable area (bar visible), distinct from fullscreen"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# A real maximize

Filed 2026-09-29 (scootbar planning). Serves **daily-drive** (a window that
fills the screen but keeps the bar, which fullscreen cannot do) and **computer
use** (an agent gets a predictable, full-size window before it screenshots).

## Where things stand

- **Fullscreen is real and stays as is** (`Super+f`, `toggle-fullscreen`,
  `set-fullscreen`): it covers the whole output including gaps, the ring and
  the bar's zone, and hides the `top` layer (`docs/configuration.md`,
  `client-fullscreen-done.md`). That is the "true full screen" option.
- **Maximize does not exist.** `docs/protocols.md` says so: no state bit is
  sent, and `set_maximized`/`unset_maximized` do nothing, from both the
  `xdg_toplevel` and `wlr-foreign-toplevel` sides. `wm_capabilities` still
  lists `maximize`, so a version 5+ client shows a maximize button that is
  inert. The config docs offer a workaround (a `1.0` entry in `column_widths`
  bound to a key), which fills the width but is not a state: nothing restores
  it, clients are not told, and it does not touch height.

## What to decide, then build

Settle the meaning first; `foreign_toplevel_management.rs` declined to invent
one, correctly. Proposed default, to confirm:

- **Maximized = the window fills the output's usable area** (the area left by
  exclusive zones, so the bar stays), full width and full height of its
  workspace strip, inside the configured gaps and ring. A per-window state in
  `scoot-core`, like fullscreen: it keeps its place in the strip, restores the
  previous width and arrangement exactly, and is not a second copy of the
  layout logic.
- **Relation to fullscreen**: fullscreen wins while set; leaving fullscreen
  returns to maximized if it was, not to the plain strip.
- **What ends it**: unmaximize, and the same events that end fullscreen
  (moved to another workspace or output, consumed or expelled); decide
  whether `cycle-column-width` / `set-column-width` end it or are ignored while
  it is set (fullscreen ignores them, `docs/configuration.md`).
- **Floating windows** (`floating-windows-done.md`): either fill the usable
  area too, or refuse; pick one and pin it.
- Respect min/max size hints (`min-size-clamp-resolved.md`): a window that
  cannot fill the area is clamped, and reports what it got.

## Surfaces

- Core action `ToggleMaximize` / `SetMaximized(id, bool)`; IPC
  `toggle-maximize` and `set-maximized ID on|off`; the snapshot field
  (`maximized`) so an agent can read it; config bind name; a default key
  (`Super+m` is free in the default table).
- `xdg_toplevel`: `set_maximized` / `unset_maximized` requests honored, the
  `Maximized` state in the configure (clients drop their rounded corners and
  shadows when told, which is the point).
- `wlr-foreign-toplevel-management`: the `maximized` state bit and the two
  requests, so a taskbar's button works. `ext-foreign-toplevel-list` has no
  state, unchanged.
- XWayland: `_NET_WM_STATE_MAXIMIZED_*` was deliberately unset because it would
  make clients change chrome for a state scoot lacked
  (`xwayland/manage.rs`); revisit once the state is real, or leave and say so.
- Remove the stale "no concept of maximized" text from `docs/protocols.md`
  and the source docs in the same PR, and document the action, key and IPC.

## Tests and edge cases

Fail-first harness tests per transport (client request, foreign-toplevel,
IPC, key). Pin: maximize then fullscreen then leave; maximize with a bar on
each edge and none; two outputs; a stacked column; a workspace switch and back;
hotplug of the window's output; a client that ignores the configure size.
Windows with no bar and windows opening while maximized keep the arrangement
byte-identical to before the change for every window that is not maximized.

## Not in this ticket

Minimize (nothing to minimize to in a scrolling layout; the requests stay inert
and say so) and `set_rectangle`.
