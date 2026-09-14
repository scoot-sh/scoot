---
title: "Client-declared `min_size` isn't clamped where it's read (MEDIUM) \u2014 DONE as item 12(b)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Client-declared `min_size` isn't clamped where it's read (MEDIUM) — DONE as item 12(b).

~~Client-declared `min_size` isn't clamped where it's read (MEDIUM)~~ —
DONE as item 12(b). One correction to the diagnosis below, found by
disabling the clamp: `ring_rects` is not the first unchecked add an
`i32::MAX` minimum reaches — `flexwm_core`'s own `World::place_workspace`
(`x + width`) overflows first, inside `arrange`, before anything renders. So
"this doesn't reach a bad write today" was true of the *write* but not of the
arithmetic: a debug build panics in the core. Original diagnosis, left as
written:
`shell.rs`'s xdg_toplevel handling reads a client's `min_size` straight
from `SurfaceCachedState` with no clamp, unlike `learned_min` (already
capped to the output's usable area). Traced the full chain to
`decorations.rs`'s `ring_rects`, which does unchecked `i32` arithmetic on
it (`rect.w + 2 * width`) — overflows for a `min_size` near `i32::MAX`,
though `clip()` currently re-bounds against the real screen size
regardless, so this doesn't reach a bad write today. The gap is real
anyway: nothing stops a future consumer of `WindowState::min()` from
inheriting the unclamped value without `decorations.rs`'s defensive
re-clip. Fix: clamp at the read site in `shell.rs`, same shape as
`learned_min`'s existing clamp.
