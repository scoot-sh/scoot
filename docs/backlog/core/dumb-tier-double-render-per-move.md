---
title: "Every pointer move renders twice ~50µs apart on the dumb tier (present-skip/vblank-retry cadence?)"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Double-render per pointer move on the dumb tier

Filed 2026-09-26 from the PR #259 review (reviewer's own log
corroboration, not the implementer's report): every pointer move renders
twice ~50µs apart on both pre- and post-fix builds (reviewer: 18
sub-2ms trailing renders amid 0.308s-paced ones; pairs are the trailing
Nones). Pre-existing, identical pixels, no failure mode observed — the
PR's hypothesis is present-skip/vblank-retry cadence.

Measure first: what schedules the trailing render (present-skip retry?
vblank fallback? cursor-only damage re-walk?), and whether the second
render is genuinely free (damage-None, post-#259 no history advance, no
present) or carries hidden cost. File the mechanism with numbers; fix
only if the cost is real.
