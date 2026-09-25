---
title: "No per-client toplevel cap: floods or chains of 10k-100k toplevels stall arrange for milliseconds to tens of milliseconds"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# No per-client toplevel cap

Filed from PR #243's review (floating windows PR 2), 2026-09-25. Not fixed
there: it predates floating windows, and the fix is a policy (a cap and
what happens at it), not a floating-layer change.

## What is wrong

A client can create as many `xdg_toplevel`s as it likes, and every one
enters the core. The costs scale with the count, on paths that run often:

- **`World::arrange` runs per frame per output** (the render path calls
  it) and per `apply()`. It is linear in windows for tiled and unrelated
  floating windows, but still multi-millisecond at a few thousand (table
  below), and floods of 10k-100k windows reach tens of milliseconds -- a
  stall per frame.
- **Floating a window with a parent runs a full `arrange`**
  (`World::parent_centre`, once per float): opening n transient windows is
  O(n^2). The reviewer's 4000-float build took 38.6 s in debug; the table
  below (a parent chain, each window floated as it opens) shows the same
  quadratic shape.
- **`recentre_floating` runs `parent_centre` -- a full `arrange` -- per
  floating window** on an output change (a resize, a rescale, an adopted
  unplugged output), so one output change with n floating windows is
  O(n^2) as well.
- Tiled opens are quadratic too (each open re-scrolls a strip whose spans
  are recomputed), less steeply.

The drawing order PR #243 added is O(n log n) and allocation-free in the
steady state; it is not what dominates here.

## Measured

Debug build, Mac (`scoot-core` only), after PR #243's drawing-order
rewrite; a temporary test (not committed) building n windows through
`WindowOpened` + (for chain/flat) `FloatingRequested` + `FrameObserved`,
then timing `arrange()` (20 calls). "chain": each window the transient of
the previous, root raised; "flat": unrelated floating windows; "tiled":
columns.

| n | kind | build (open + float all) | one `arrange` |
|---|---|---|---|
| 250 | chain | 47.6 ms | 0.41 ms |
| 250 | flat | 1.1 ms | 0.20 ms |
| 250 | tiled | 14.7 ms | 0.21 ms |
| 500 | chain | 112 ms | 0.66 ms |
| 500 | flat | 2.1 ms | 0.32 ms |
| 500 | tiled | 48.8 ms | 0.36 ms |
| 1000 | chain | 448 ms | 1.41 ms |
| 1000 | flat | 5.9 ms | 0.64 ms |
| 1000 | tiled | 195 ms | 0.72 ms |
| 2000 | chain | 1.80 s | 2.91 ms |
| 2000 | flat | 16.1 ms | 1.27 ms |
| 2000 | tiled | 780 ms | 1.42 ms |
| 4000 | chain | 7.26 s | 6.06 ms |
| 4000 | flat | 53.9 ms | 2.62 ms |
| 4000 | tiled | 3.15 s | 3.06 ms |

(The reviewer's own table, debug, at n = 250...4000 for the same three
shapes, was not carried in the message that filed this; these are this
PR's re-measurement. The reviewer's 38.6 s 4000-float build predates the
drawing-order rewrite.)

## What to do

- A per-client cap on live toplevels, modelled on the popup cap
  (`popup_parent.rs`, `MAX_POPUP_DEPTH` = 64) and the subsurface cap
  (`subsurface_depth.rs`, `MAX_SUBSURFACE_DEPTH` = 64): decide the number
  (real clients open a handful; a browser with many windows, dozens),
  and whether a toplevel past it is refused with a protocol error (a
  client kill, which the fd and popup bounds already accept for abuse) or
  kept out of the core.
- Independently: `parent_centre` should not need a whole `arrange` to find
  one parent's rect (a single-window placement query, like
  `World::floating_geometry` for floating parents and a strip query for
  tiled ones), and `recentre_floating` should compute the arrangement once
  for all the windows it re-centres.

## Evidence expected

The table above re-measured before/after; a harness test that a client
opening past the cap gets the chosen answer and other clients are served.
