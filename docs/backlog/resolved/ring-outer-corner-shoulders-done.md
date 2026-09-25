---
title: "Focus ring outer corners have square shoulders: side bars paint over the outer arc — DONE"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Ring outer-corner shoulders — DONE

RESOLVED 2026-09-24, in the same PR as
[`clip-to-committed-size-done.md`](./clip-to-committed-size-done.md). The
ticket as filed is kept verbatim below.

## Resolution

The mechanism was as filed. The rounded path reused `ring_rects`' side
bars, which span the window's full height, so from the window's top edge
down they filled the outer corner square that the painted strip left
transparent. `ring_rects` is unchanged: the square ring needs full-height
bars, and its tests pin them.

The rounded path now builds its side bars with `painted_side_bars` from
exactly what the strips were painted and placed from: the strip canvases,
the top strip's origin, and the full-canvas `inner`/`outer` rects. They
cover the band's columns, on rows from where the top strip's element ends
to where the bottom strip's starts. They live in their own persistent
buffers on `PaintedRing`, sized in physical pixels and drawn at scale 1.
`WindowRing`'s logical buffers are left to the square path and the
fallback, so no buffer holds a size in two units. The strips are at least
`thickness + radius_outer` rows tall, so every row a bar covers is straight
on both edges. A sweep over six scales, four thicknesses and 16 sub-pixel
positions pins this, and a too-short window gets no bars.

One side effect: a translucent ring colour used to blend twice where the
strips overlapped the bars. It no longer overlaps. Opaque colours, all the
defaults, are unchanged. The ring still draws four elements per window.

## Evidence

Every existing `check_ring_content_alignment` caller now also asserts the
outer edge. For each row of the outer corner band, top and bottom, left and
right, the cut pixels are background and the first kept pixel is ring, at
radius `radius + thickness_phys` around the ring's own outer rect. On
`b755f11` (main plus tests) this failed at 1.0, 1.25, 1.5 and 2.0 under
both pixman and GLES (`top outer row 4/5/6/8, cut on the left`). It passes
at `7164226`. The radius-1 caller at 1.5 has no affected rows. Live on
default foot: outer arc 11/21 → 21/21 rows at 1.5 and 8/14 → 14/14 at 1.0,
headless and `--tty`, the ticket's rows 18–27 and 12–17 exactly. See the
table in the sibling record.

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
