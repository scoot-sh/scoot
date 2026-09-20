---
title: "Rounded window corners — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Rounded window corners — RESOLVED

## What it said

Compositor-side rounding via `[appearance] corner_radius`, sequenced behind
the `scootctl` split and milestone 6. Its cost framing: the cost is not the
corners but the opacity — a rounded window no longer covers its rectangle, so
what is behind it must composite. Measure with stacked, overlapping windows;
a single-window benchmark shows nothing. Milestone 6 (GLES) changes the
calculus: a reasonable outcome is renderer-aware (rounded on GPU,
square-or-opt-in on pixman). Things that must follow the radius: the focus
ring, damage tracking, capture, and a decision (not a default) on popups.

## Resolution

Shipped: `[appearance] corner_radius` (logical px, default `0` = square),
live-reloadable, with the ring following the radius and popups staying
square. Three findings on the way, one correcting the ticket's decomposition
and one reversing an implementation:

- **The ticket's decomposition was incomplete, and the missing half is the
  load-bearing one.** The plan was "damage stays the bounding box, change the
  opaque region." Verified against the pinned Smithay source, the opaque
  machinery is real (`backend/renderer/damage/mod.rs`: elements fully hidden
  behind accumulated opaque regions are skipped, and damage under opaque
  regions is subtracted). But changing the opaque region alone changes no
  pixel — the window still paints its full rect — while pointlessly
  compositing what is below. And the draw clip alone is a
  persistent-artifact bug: the tracker trusts the full-rect opaque claim and
  never redraws below the cut corners, so they show stale pixels. The two
  ship as one atomic change or not at all: each window element is wrapped in
  `rounded::Rounded`, which cuts the corner staircases out of every draw
  *and* shrinks the opaque region to match, both from the same radius. Found
  the hard way: the first pixel run showed full-square corners with 32 stray
  pixels cut mid-window, which traced to `draw` receiving element-relative
  damage while the filter subtracted output-space rects
  (`render_output_internal` translates damage by the element geometry before
  calling `draw`).
- **No Smithay rounded-corner support exists at the pinned rev, and pixman
  cannot do a mask through the renderer-agnostic `draw`.** So the cut is a
  per-row staircase (`cut_width`, pixel-center rule, closed form pinned
  against an independent per-pixel oracle on radii 0–64 plus spot larges),
  identical rects on pixman and GLES — one implementation, byte-identical
  pixels on both renderers (the whole pixel suite passes under
  `SCOOT_TEST_RENDERER=gles` unchanged). `radius = 1` cuts nothing (the
  corner pixel's center is still inside the unit circle): the rule applied
  uniformly, pinned end to end. The ring is two painted strips (top/bottom,
  via `MemoryRenderBuffer`) plus the two solid side bars — the same four
  elements per window as the square path.
- **The first ring design failed on numbers and was replaced.** A
  full-window painted ring (transparent middle) measured +67%/+91%/+48%
  medians (single/tiled3/overhang3, pixman, 6 alternating runs): the
  transparent middle's full-window blend dominated everything. The strip
  redesign cut transparent pixels per frame ~150x (from ~300K to ~2K on the
  test scene). Lesson recorded: the ticket priced opacity loss, but the
  implementation's own overdraw was the larger cost until measured.

## Mechanism (for the reviewer)

- `Rounded<E>` wraps one `WaylandSurfaceRenderElement` from the window's
  toplevel tree: same `id`/`commit`/`geometry`/`damage` (damage stays the
  bounding box, correct by construction — the draw clip applies on every
  draw no matter whose damage it is), opaque minus the four corner squares
  (a superset of the drawn cut, which is the direction that keeps the claim
  sound), draw with damage minus the staircase rows (converted to
  element-relative via `dst`, scratch `Vec` reused across frames).
- Gathering splits each window's elements the way `Window`'s own
  `AsRenderElements` does (popups first, unwrapped; toplevel tree wrapped),
  in the same order/filter/positioning as `render_elements_for_region`
  (placements reversed, bbox filter, render location minus region). Radius 0
  keeps calling `render_elements_for_region` and the four solid bars, so the
  default session is byte-identical by construction, not by care.
- Clip and ring share `clip_rect` (from the arrangement placement, not the
  drawn surface — stable across resize lag, and it cuts stale overhang to
  the placement rather than bleeding into the gap) and `cut_width`, so the
  ring's inner edge and the window's outer edge coincide exactly.
- Effective radius is per window (`min(configured, min(w,h)/2)`); absurd
  values become stadiums. Uniform on all four corners including screen
  edges — no special-casing (the layout's gap insets every window anyway).

## Numbers (release, dev VM, 800x800, 200 frames x 12 alternating runs/tier)

pixman:

| scene | square min/median/max | radius 12 min/median/max | median delta | paired min/median/max |
|---|---|---|---|---|
| single | 332/383/445µs | 353/407/468µs | +6.3% | -6.4/+8.5/+32.4% |
| tiled3 | 582/657/760µs | 682/732/821µs | +11.4% | -0.7/+8.9/+29.3% |
| overhang3 (2x buffers) | 896/1013/1060µs | 784/998/1089µs | -1.6% | -20.8/+1.1/+12.8% |

gles (software llvmpipe — 15–25x slower baseline, not a GPU):

| scene | square min/median/max | radius 12 min/median/max | median delta | paired min/median/max |
|---|---|---|---|---|
| single | 3.76/4.06/4.28ms | 4.83/5.09/5.49ms | +25.4% | +13.1/+28.6/+34.9% |
| tiled3 | 6.50/6.71/7.41ms | 7.92/8.78/9.64ms | +30.9% | +20.8/+29.3/+42.9% |
| overhang3 | 8.48/9.05/10.05ms | 9.31/10.36/11.03ms | +14.5% | +4.5/+11.8/+22.4% |

Methodology notes: tiers alternate within each run; minima are the
least-polluted runs (noise only adds time), paired deltas defeat
between-run drift. Ranges overlap on pixman — the effect is small and the
honest reading is "single-digit µs to tens of µs per frame, ~0% on the
overlap scene." The one directional signal that survived all runs: the
overhang scene's minima favor rounded (896→784µs) — the clip removes the
bleed it would otherwise composite, exactly the overlap-removal the design
section claims. Absolute scale matters more than the percentages: +60µs on
a 660µs test frame is 0.35% of a 16.7ms 60Hz budget.

## The GLES decision

No renderer gating, on numbers and scope both. Pixman (the default path)
costs ~+9% medians on the realistic scene, ~0% on the overlap scene, and
exactly nothing at the default `0` — too small to gate by renderer, and the
ticket's "square-or-opt-in on pixman" assumed a cost an order of magnitude
larger. The GLES numbers above are llvmpipe's per-draw CPU overhead, not a
GPU's: the staircase is a few dozen extra scissored quads, which is noise
on real hardware and unmeasurable on this project's GPU-less dev VM (the
same caveat milestone 6's record states). Gating on those numbers would be
deciding on an artifact. What ships instead works on both renderers through
the shared mechanism — no shader, no divergence — with the default `0`
leaving every session byte-identical. If real-GPU hardware ever shows a
staircase cost worth caring about, *that* measurement can gate it.

## Follow-ons (deliberately out)

- GLES shader path with a smooth (AA) mask: the staircase's edges are
  1px-stepped. Needs renderer-specific code; the ticket's GPU-tier
  reasoning applies to it, not to this PR.
- Popup/menu rounding: popups stay square by decision, recorded in
  `docs/configuration.md`.
- Per-window radius, animated radius transitions: no config surface for
  either, by the no-speculative-knobs rule.
