---
title: "Focus ring outer corners have square shoulders: side bars paint over the outer arc"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Ring outer-corner shoulders

Filed 2026-09-24 from the gh #205 re-verification. At every scale, the
ring's *outer* corner is only rounded in the ring rows above/below the
window; below the window's top edge the solid side bars fill the corner
square (at 1.5: rows 18–27 cut 0 px where a concentric outer arc cuts 6→1;
at 1.0 rows 12–17 cut 0 where it cuts 4→1). Likely `ring_rects` in
`decorations.rs` (~393–394): side bars span the full `rect.h`, painting over
what the strips leave transparent. Predates #207 (`ring_rects` unchanged
since `d5c914f`). Serves **daily-drive** (visual polish; `CLAUDE.md` ranks
it below engineering, so the fix must not cost per-frame work).

Fix: side bars span only the straight part (between the arcs), and the
strips carry the full outer arc; pixel test that the outer edge is a
concentric arc at 1.0, 1.5, 2.0 on pixman and GLES.
