---
title: "Translucent bar: measure the compositor-side recomposite cost under animation"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
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

If a configuration exceeds the budget, record the options: raise the
shipped looks' opacity, or document the cost next to the `[opacity] bar`
option.

## Not in this ticket

Changing any look's bar opacity (that decision lives with the looks), and
the shadow and entrance work, which are
[scootbar-depth-tokens](scootbar-depth-tokens.md) and
[scootbar-popup-entrance](scootbar-popup-entrance.md).
