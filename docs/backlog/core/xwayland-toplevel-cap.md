---
title: "No per-client XWayland toplevel cap: X windows enter the core uncounted"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# No per-client XWayland toplevel cap

Filed 2026-09-26 from the per-client xdg-toplevel cap
(`docs/backlog/core/per-client-toplevel-cap.md`). That cap counts only
`xdg_toplevel`s, at `State::add_window`: managed X windows enter through
`map_x11_window`, which bypasses `add_window` and neither claims nor
releases. So one X client can still open toplevels without bound, and every
one still enters the core with the same per-frame `arrange` cost the xdg cap
bounds.

Low because the X socket is the session's own XWayland instance, not the
open Wayland socket: reaching it takes a client the session already runs
(or its `$DISPLAY`), where the xdg cap guards against any sandboxed
Wayland client at all. The xdg cap's number (128) and refusal form
(`wl_display.no_memory`) are the shape to copy, but X has no client object
to post an error to -- the fix needs its own refusal (refuse the map,
like the insane-frame-extents refusal already in `map_x11_window`) and its
own per-X-client counting. Serves daily-drivability (a runaway X app
should not stall the desktop) more than computer use.
