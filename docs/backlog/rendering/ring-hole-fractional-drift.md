---
title: "Fractional-scale ring-hole drift in the painted-ring origin refresh"
status: "open"
area: "rendering"
priority: "low"
blocked: null
---

# Fractional-scale ring-hole drift in the painted-ring origin refresh

Filed 2026-09-20 from `scoot-reviewer`'s re-review pass on PR #184, which
landed position-refresh for the painted ring strips and recommended this
as the follow-up rather than scope for that PR.

## Mechanism (verified by brute force, not observed live)

`refresh_strip_origins` (`crates/scoot/src/compositor/decorations.rs`)
compares only strip canvases to decide buffer reuse, but paint content
(`inner`/`outer` in-canvas offsets) also depends on absolute position
through rounding phases. Concrete instance from the reviewer's replica
(scale 1.25, thickness 2, 100px window, x=1→2): canvases identical
`(130,17)/(130,16)` → no repaint, but `inner.x` moves 2→3, so the reused
buffer lands the ring hole **1px off the window clip** until a key change
(color/size) repaints. Integer scales: 0 drift in 1.3M+ move pairs per
scale. Fractional (1.25/1.5/1.75/1.33): 25–55% of canvas-matching move
pairs drift.

Harm is bounded: ≤1px, fractional-scale sessions only, self-heals on the
next focus change or resize (color/size are in the key). Never stale,
never a wedge — which is why it ships as a follow-up, not a blocker.

## Fix shape (reviewer's, cheap and complete)

Compare `plan.inner`/`plan.outer` too in the reuse check — still
allocation-free. `plan_strips()` already computes the full plan, so this
is one more comparison, not new geometry.

## Proof shape

Extend the brute-force replica (rounding replicated from pinned Smithay
`geometry.rs`) or drive fractional-scale move pairs live and assert hole
alignment; plus the standard full cheap set (`nextest`, `clippy`, `fmt`,
smoke). No benchmark owed (same steady-state shape, one more compare).

## Out of scope

Production behavior beyond the reuse check (the ring path itself is
correct), any other test, CI changes.
