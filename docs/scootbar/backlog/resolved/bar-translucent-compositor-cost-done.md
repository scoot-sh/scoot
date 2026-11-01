---
title: "Translucent bar: measure the compositor-side recomposite cost under animation"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-01"
---

# Translucent bar: measure the compositor-side recomposite cost under animation

Filed 2026-11-01 from the review of PR #850
([bar-opacity-benchmark](resolved/bar-opacity-benchmark-done.md)), child of
GitHub epic [#807](https://github.com/scoot-sh/scoot/issues/807) (section 4).
Serves **daily-drive**: the five shipped looks now ship a translucent bar
(`[opacity] bar` 0.7 to 0.88), and the CPU tier is the one that pays if
translucency costs more than the measurements say.

## The gap

The benchmark that gated the lift measured the bar's own paint cost (the
same 15 to 60 us at every alpha, because `fill_shaped` computes one
premultiplied pixel and runs the same copy loop), idle wakeups and popup
cycles. The compositor-side cost of a translucent layer surface was
inferred, not measured. The reviewer's reasoning, which holds for the
shipped default:

- The bar has an exclusive zone (`crates/scootbar/src/daemon/surfaces.rs`
  `set_exclusive_zone`), so no tiled window sits under it; only the
  wallpaper does, and a static wallpaper adds no per-frame cost.
- The bar redraws on revision-driven damage, not on background changes, and
  the compositor recomposites from the cached bar buffer.
- `Style::opaque_inset()` returns `None` below alpha 255 and
  `set_opaque_region(None)` clears the region, so the compositor correctly
  gives up occlusion culling for the bar.

What no run covers: the `frames` bench is screenshot round trips with a
resolution floor set by PNG encoding and IPC (about 10 ms medians), and no
run animates anything under the bar. Cases that put moving pixels behind a
translucent bar are:

- an animated wallpaper (scootbg animated images, blocked on
  `animated-images`);
- a floating window dragged or slid under the bar's rectangle;
- a window sliding in from the top edge during a workspace switch on a
  top bar;
- the lock screen or an overlay over the bar.

## What to do

Put a number on the non-default configurations on the CPU (pixman) tier:

1. A micro-bench of the compositor's blend of a 1920x40 translucent strip
   against a plain blit (pixman `OVER` with a constant mask versus `SRC`),
   at 1x and 2x, release build, `taskset`, median and p5 over interleaved
   samples.
2. A headless `scoot --renderer cpu` run with a floating window moving
   under the bar (via `scoot msg` actions or the virtual-input path),
   comparing frame time with the bar at alpha 1.0 and at each example
   opacity, using the compositor's frame instrumentation if it has any,
   otherwise a controlled screenshot cadence.
3. State the budget before measuring: translucent frame time within an
   agreed margin of opaque, per frame, at 1x and 2x.

## Budgets (stated 2026-11-01, before measuring)

Stated before any number below was taken, relative to the opaque
(`bar = 1.0`) measurement in the same scenario, on the same build, on
the same machine. The +10% follows the bar-opacity gate's precedent
(`resolved/bar-opacity-benchmark-done.md`: D15 accepted ~+5-6% per
frame for rounded corners, and the opacity gate allowed +10% for VM
noise; the bar's own fill is alpha-independent by construction, so the
compositor-side blend is the unknown this ticket exists to measure).

1. **Micro-bench** (compositor blend of a 1920x40 strip, plain blit vs
   blend, at 1x and 2x): translucent median and p5 within +10% of
   opaque, per scale.
2. **Headless run** (floating window moving under the bar, frame time
   at bar alpha 1.0 vs each example opacity 0.7, 0.78, 0.82, 0.88):
   translucent frame time within +10% of opaque, per frame, at 1x and
   2x.

Verdict rule: every configuration inside budget keeps the shipped
looks' opacities as they are and this ticket records the measured
cost; any configuration outside budget records the options (raise the
shipped looks' opacity, or document the cost next to the `[opacity]`
bar option) and implements ONLY the documentation option in the same
PR.

If a configuration exceeds the budget, record the options: raise the
shipped looks' opacity, or document the cost next to the `[opacity] bar`
option.

## Not in this ticket

Changing any look's bar opacity (that decision lives with the looks), and
the shadow and entrance work, which are
[scootbar-depth-tokens](resolved/scootbar-depth-tokens-done.md) and
[scootbar-popup-entrance](resolved/scootbar-popup-entrance-done.md).

## Resolution (2026-11-01, verdict: OVER BUDGET on the micro-bench, documented)

The compositor-side cost is real and measured: a translucent bar strip
costs ~2.3-4.4x its opaque blit per redraw on the CPU (pixman) tier, at
both scales. The shipped default (exclusive zone, static wallpaper) still
pays nothing per frame -- no tiled window sits under it -- so no look
changes here; the cost is documented next to the `[opacity]` bar option
in `site/src/content/docs/scootbar/configure.md`, in the same PR.

### What was built

- `crates/scoot/src/compositor/headless/bench.rs`
  `translucent_strip_blend_cost` (in-crate, `#[ignore]`d): the real
  `PixmanRenderer::render_texture_from_to` path with the real formats
  scootbar commits (`daemon/canvas.rs`: `Argb8888` when
  `Style::translucent()`, else `Xrgb8888`) and the real opaque regions it
  sets (whole strip when opaque, `None` when translucent), over a
  1920x40 strip at 1x and 2x, median + p5 over 300 interleaved samples
  per cell, best of 2 passes. Verified against the pinned Smithay source
  (fork rev `b65cbc5`, `pixman/mod.rs` `render_texture_from_to`): the
  backend picks `OVER` vs `SRC` solely from the source format
  (`has_alpha`) and ignores `_opaque_regions`; the opaque region matters
  one layer up, where the damage tracker skips what opaque regions cover.
- `crates/scootbar/tests/translucent_cost_bench.rs` (`underlay`,
  integration, `#[ignore]`d, `BENCH key=value` lines, the
  `opacity_bench` pattern): headless `scoot --renderer cpu` with a clock
  bar at `exclusive = false` (an overlay-style bar, so a floating window
  can sit behind its rectangle) at alpha 1.0 and each example opacity,
  a spawned `foot` floated and stepped 24 times from above the bar
  through it, one timed `screenshot` round trip per step (the compositor
  has no frame instrumentation over IPC, so a controlled screenshot
  cadence). A placement guard prints the window's placed rect after the
  last step and asserts it overlaps the bar's rows.

### How it was measured

Release builds on the dev VM (`taskset -c 2`, 6 vCPU shared box, load
0.1-1.0, two other agents' idle `scoot --headless` sessions present but
no competing builds during the timed runs), `SCOOTBAR_REQUIRE_SCOOT=1`
with `scoot` built beside the bar from the same tree; opaque and
translucent are the same binary (opacity is a runtime `[bar]` value).
Micro: 3 invocations, rotating sample order per round. Live: 2 chains
(the second with the placement guard), each rotating the opacity order
per run (24 timed steps per config per scale).

The VM's CPU swings between speeds (opaque baselines move ~2x run to
run), so configs are compared within interleaved runs, with the p5
(which the bursts leave alone) alongside the median.

### Numbers (medians; p5 beside them; full per-run numbers in the PR)

Micro, per strip redraw (76,800 px at 1x; 307,200 px at 2x), us:

- 1x opaque (XRGB8888/SRC): 29.6/9.5, 20.8/17.5, 16.0/10.8 across runs.
- 1x translucent (ARGB8888/OVER): 70.7/22.8, 51.9/41.0, 70.4/31.8.
  Within-run deltas: +41.1/+13.3, +31.1/+23.5, +54.4/+21.0 us
  (+139%/+140%, +150%/+134%, +340%/+194%).
- 2x opaque: 76.3/59.2, 116.2/55.5, 79.2/58.7.
- 2x translucent: 193.0/144.2, 283.9/129.2, 190.2/142.0.
  Within-run deltas: +116.7/+85.0, +167.7/+73.7, +111.0/+83.3 us
  (+153%/+144%, +144%/+133%, +140%/+142%).

Live, floating window stepping under the bar, screenshot ms per step
(n=24 per cell; window_y=32 h=500 in all 20 guarded configs, so the
underlap is real):

- 1x opaque: 15.29/13.51, 16.68/6.69, 17.68/13.30, 15.36/11.33.
- 1x translucent medians range 13.72-18.94 across alphas and chains
  (-15%..+22% vs opaque within each chain, signs flipping).
- 2x opaque: 14.78/11.55, 17.54/11.42, 18.29/14.25, 14.30/7.90.
- 2x translucent medians range 14.22-20.55 (-12%..+44%, signs flipping).

### Verdict per budget

1. Micro-bench: FAIL. Translucent is +133% at best (2x p5) and up to
   +340% (1x median outlier run; +194% p5 in the same run) outside the
   +10% budget, at both scales, in all 3 runs, median and p5 alike.
   Expected in hindsight: `SRC` writes each pixel once while `OVER`
   reads the destination and blends, and the translucent bar also gives
   up the damage tracker's occlusion culling for its whole strip.
2. Headless run: below the metric's resolution floor, as the prior
   gate's `frames` bench was. The ~40-170 us strip blend is under 1.5%
   of the ~15 ms screenshot round trip (PNG encode + IPC), opaque
   itself swings +-20% run to run, and every translucent range overlaps
   opaque's fully with signs flipping between chains. No systematic
   offset is resolvable at this cadence; the micro-bench above is the
   resolving measurement.

### Options recorded, decision taken

- Raise the shipped looks' opacity (back toward 1.0): NOT taken here --
  that decision lives with the looks, and the shipped default pays no
  per-frame cost anyway.
- Document the cost next to the `[opacity]` bar option: TAKEN, in the
  same PR (`site/src/content/docs/scootbar/configure.md`, "Shape,
  opacity and depth"). No look's opacity changed.

No hot-path code change: the pixman backend behaves as designed
(format-driven `OVER` vs `SRC`, region-driven culling), so there is no
compositor bug to file; the cost is inherent to blending.
