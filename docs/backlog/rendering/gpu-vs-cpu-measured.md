---
title: "CPU vs GPU rendering has never been measured on a real GPU"
status: "open"
area: "rendering"
priority: "high"
blocked: "needs the user's Asahi machine — no VM or container can answer it"
---

# CPU vs GPU rendering has never been measured on a real GPU

Filed 2026-09-19. Milestone 6 built a GPU renderer and a GPU scanout tier,
and **every performance number attached to either is a software
rasteriser's.** The dev VM's EGL device answers `is_software() == false` and
is then served by llvmpipe, so the numbers are real measurements of the
wrong thing.

The runbook is [`Asahi.md`](../../../Asahi.md)'s Test 4. This entry exists so
the unanswered question is visible in the backlog rather than only in a
hardware document.

## What is actually known

| measured on llvmpipe | vs pixman |
| --- | --- |
| stage 2 — GLES offscreen, frame read back | **17–32x slower** |
| stage 3B — GPU scanout | **~1.5x slower** |

Both on the same rasteriser, so the comparison between them is sound even
though neither is a GPU number. Removing the read-back closed almost the
entire gap, which says the read-back — not the rasterising — was the
dominant cost. On hardware where rasterising is the GPU's job, scanout
should therefore win.

**That is an extrapolation.** It is a reasonable one and it is the reason
the tier was built, but nothing in this repository measures it.

## Why FPS is the wrong headline

scoot renders on damage, not on a clock. "Frames per second" mostly reports
what the clients asked for. For a compositor whose pitch is *lightweight*,
the numbers that decide whether the GPU tier is worth using are:

1. **Idle CPU** — a compositor that is fast but busy is worse on a laptop
   than one slightly slower that sleeps. Jiffies over a fixed quiet window.
2. **Frame cost under damage** — where scanout should show its advantage,
   since the read-back is exactly what it deletes.
3. **RSS** — a GBM swapchain of several buffers at panel resolution is real
   memory that the dumb tier does not spend.
4. **Power** — GPU scanout may cut CPU wakeups and raise GPU draw. The net
   is genuinely unknown and is the most interesting number available.

## Methodology, which is not optional here

Alternate the tiers; do not run each once. This project has already been
burned: a single A/B pair read as a **70% regression** that was pure noise,
and it took eight alternating rounds to establish the medians were 2.4µs
apart against a 19µs per-build spread. Report medians and spread, never a
best-of, and confirm which tier is live (`scanout="gpu"` in the log) before
trusting any figure — `--renderer gles` without `--features gpu-scanout`
silently keeps pixman.

## Expect correctness before performance

Scanout has never run on a real GPU, drives the **primary plane only**, and
this hardware is the split render/display case (AGX has the render node,
`apple,dcp` owns the connectors) which the design allows for but nothing has
exercised. If it does not come up at all, that is a more valuable result
than any timing, and it belongs in `docs/roadmap/06-gpu-pipeline.md` next to
the caveats that predicted it.
