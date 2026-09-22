---
title: "CPU vs GPU rendering, measured on a real GPU (RESOLVED 2026-09-21)"
status: "resolved"
area: "rendering"
priority: "high"
blocked: null
---

# CPU vs GPU rendering, measured on a real GPU

Filed 2026-09-19 because milestone 6 built a GPU renderer and a GPU scanout
tier and **every performance number attached to either was a software
rasteriser's**. Answered 2026-09-21 on the user's Apple M2 (`apple,t8112`)
under Asahi Linux, `eDP-1` at `2560x1600@60`, `scale = 1.5`, on battery.

Runbook: [`Asahi.md`](../../../Asahi.md)'s Test 4, now runnable as one
command (`scripts/asahi-test4.sh`, which writes its own report). Full
evidence with method and caveats:
[`docs/roadmap/06-gpu-pipeline.md`](../../roadmap/06-gpu-pipeline.md),
"Evidence (Apple M2 / AGX under Asahi Linux, 2026-09-21)". Two runs -- four
alternating rounds at `650a187`, then two at `52672b8` closing two gaps the
first left.

## What it said, and what the answer was

| the VM measured (llvmpipe) | the M2 measured |
| --- | --- |
| offscreen GLES, frame read back: **17-32x slower** than pixman | **parity**: 55.0 vs 56.4µs (empty), 57.3 vs 56.0µs (8 windows) |
| GPU scanout: **~1.5x slower** than the dumb tier | **4.2-5.1x faster**: 0.455 -> 0.090 j/ev motion, 3.34 -> 0.797 j/ev relayout |

The entry's own reasoning was that the read-back, not the rasterising, was
the dominant cost, so scanout should win where rasterising is the GPU's job.
That held, and the margin is larger than the VM's shape suggested.

Against the four metrics this entry named, in its order:

1. **Idle CPU** -- 0 jiffies over 10s, both tiers, every round. Neither
   wakes when nothing moves, so the "fast but busy is worse on a laptop"
   concern does not apply. Idle power identical at 5.52 W.
2. **Frame cost under damage** -- the table above; 20.8% of one core down to
   4.2% under motion, 51.9% down to 14.0% under relayout.
3. **RSS** -- +7 to +17 MB for the GBM swapchain (the baseline moves between
   runs, so it is a range). The one metric the dumb tier wins.
4. **Power** -- called "genuinely unknown and the most interesting number
   available" when this was filed. Scanout draws **less**: 0.22 W lower
   under motion, 0.25 W lower under relayout, whole-system, ~3.5% of draw.
   It does not trade CPU wakeups for GPU draw.

## Methodology, which this entry insisted on and which changed

Alternating rounds, medians with spread, never a best-of -- kept. Two things
had to change for this hardware, both found by rehearsing the harness rather
than by the run:

- **The VM's "300 unpaced pointer moves" measures nothing here.** An IPC
  round trip costs ~0.8 ms on this machine against the VM's ~11 ms, so an
  unpaced burst arrives ~20x faster than the panel refreshes and the
  compositor correctly coalesces it away: 3000 moves produced 16 jiffies.
  Damage is now driven at a fixed rate *below* the refresh rate for a fixed
  wall-clock window, and normalised per event -- the tiers get through
  different event counts in the same window (156 vs 176 relayouts), so a
  per-round total would compare different amounts of work.
- **"Confirm which tier is live before trusting a number" was necessary and
  nearly failed.** The compositor coloured its log unconditionally, so
  `scanout="gpu"` in a redirected log is really
  `scanout\x1b[0m\x1b[2m=\x1b[0m"gpu"`; the harness reported the tier as
  absent on a run where it had come up. Fixed at the source (`225719e`,
  gated on `IsTerminal`) -- every `tee` capture `Asahi.md` recommends was
  affected.

## What stays open

- The motion scene is *large-bbox* damage (the injected path jumps across
  ~900x600 logical pixels). A small cursor-rect move is a different
  measurement and was not made.
- One panel, one resolution, one machine.
- Cursor and overlay planes: phase 2 of
  [gpu-scanout-planes](../rendering/gpu-scanout-planes.md), which phase 1
  of this run unblocks.
