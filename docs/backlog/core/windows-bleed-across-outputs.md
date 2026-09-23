---
title: "Windows are not clipped to their output: an off-edge column draws onto — and takes clicks on — the neighbouring monitor"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# Windows bleed onto the neighbouring output

Filed 2026-09-23 by the PR #223 re-review. Serves **daily-drive**
(multi-monitor) and **computer use** (a click an agent aims at monitor 1 can
land on a window that belongs to monitor 2). Pre-existing, made much worse
by fullscreen.

## What is wrong

Nothing clips a window to the output it is placed on:

- Both render paths gather everything in the space overlapping the output
  (`render_elements_for_region(region)`; `rounded_window_elements` clips only
  to the window's own layout rect) — `crates/scoot/src/compositor/render/elements.rs`.
- `window_under` (`state.rs`) uses `space.element_under` with no output
  filter, so input follows the drawn bleed.
- `apply()` maps output 2's windows after output 1's, so a bleed sits *on
  top of* output 1's own windows and takes their clicks.

Live repro (`--headless --outputs 2 --width 800 --height 600`, output 2 at
x=800): two `foot` windows on output 2, fullscreen the left one, then
`focus-column right` → `{"id":1,"output":2,"x":394,"w":800,...}` and output
1's screenshot shows window 1 across x 394–800. Tiled columns bleed the same
way (two 0.667-width columns on output 2: window 1 at x:550 w:513 is drawn,
ring included, on output 1) — fullscreen only widens it to a whole output.

## What to do

Clip each window's elements (and its ring/decorations) to its placement's
output rect, on every render path (screen, screenshot, screencopy, both
renderers), and restrict hit-testing the same way so a window only receives
input inside its own output. Watch the cost: this is the per-frame gather
and the per-motion hit test — no per-frame allocation, benchmark before and
after. Correct the "never over it" wording in `docs/protocols.md`, the
`ipc.md` `fullscreen` row and the `WindowSnapshot` doc (true only on the
window's own output today), and drop or source the unsourced "(niri answers
the same way.)" in `protocols.md` / the `above_windows` doc.

## Evidence

Harness tests with two outputs: pixel readback of output 1 shows none of
output 2's windows (tiled overhang and focused-away fullscreen), and a
click/motion at the bled pixels reaches output 1's window (or nothing), not
output 2's. Fail-first against current main.
