---
title: "A `present()` skipped for an in-flight flip consumes that frame's damage; the retry renders with damage `None`"
status: "open"
area: "rendering"
priority: "low"
blocked: null
---

# A `present()` skipped for an in-flight flip consumes that frame's damage

Found by code reading during the isolate-first probe that closed
[`session-lock-surface-not-drawn-live`](../resolved/session-lock-surface-not-drawn-live-done.md)
as unreproduced — observed never, live or in-harness. Filed so the trace
doesn't evaporate.

A `present()` skipped for an in-flight flip consumes that frame's damage
in `render_output`, and the `present_skipped` retry re-renders with damage
`None` (`headless.rs`, `tty/mod.rs` — the exact lines are in the resolved
record above), so scanout keeps stale pixels until unrelated damage
arrives. It cannot explain black screenshots (the read-back image is
always correct — only scanout goes stale), it needs a commit landing
inside a ~16ms flip window (a 30-commit burst at 5ms spacing produced 0
skips on the dev VM: frame timer and vblank run phase-locked there), and
it self-heals on the next damage. The candidate fix (`invalidate_ages` on
skip) would touch the hot present path plus the `blank_flip` interplay
from PR #84 for an unobserved failure — correctly out of scope until
observed. Revisit if a stale-scanout frame is ever captured live; the
discriminating evidence would be scanout (photons or cast) differing from
a same-instant `msg screenshot` after a commit in a flip window.
