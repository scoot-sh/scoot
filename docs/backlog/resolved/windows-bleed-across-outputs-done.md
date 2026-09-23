---
title: "Windows are not clipped to their output: an off-edge column draws onto — and takes clicks on — the neighbouring monitor — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Windows bleed onto the neighbouring output — RESOLVED

RESOLVED 2026-09-23 (PR #224). A window is drawn and takes input only on the
output it is placed on; the rule and where it is applied are in
`crates/scoot/src/compositor/output_clip.rs`'s module doc:

- **Drawing**: each output's frame gathers only the windows placed on it
  (`render/elements.rs`'s `window_elements`, replacing
  `Space::render_elements_for_region`, whose region-overlap test *was* the
  bleed), in that output's own coordinates, on every path that draws a
  frame (screen, screenshot, screencopy; pixman, offscreen GLES and the
  scanout tier all go through the same gather). The ring
  (`Decorations::elements`/`elements_rounded`) is filtered the same way.
- **Input**: `State::window_element_under` (the pointer focus search via
  `window_under`, and click-to-focus in `input.rs`) tests a point only
  against the windows placed on the output the point is on, reading an
  output stamp `apply()` writes beside `map_element`. Over no output,
  nothing is hit. Fast path is Smithay's own `element_under`; only a point
  under another output's overhang re-walks the stack.
- **Popups go with their parent**: a menu crossing a shared edge is cut
  there, as it already was at an output's outer edge (scoot applies no
  positioner constraint adjustment -- filed as
  [popup-constraint-adjustment](../core/popup-constraint-adjustment.md)).

**Found beyond this ticket's text, and fixed with it:** the focus ring and
the rounded-corner clip were built in *global* coordinates while each
output's framebuffer starts at its own origin, so on every output but the
first the ring landed a whole output to the right (off-screen: output 2
never showed a ring) and rounding cut the wrong pixels. Visible in the
before screenshots below (`A-output2.png`: the focused window has no ring).
Both now build output-local (`output_clip::to_output_local`); pinned by
`the_second_output_draws_*_like_the_first`, which composites the same lone
window on either output and requires byte-identical frames (square,
rounded, scale 2 and 1.5).

Left out, filed as
[output-membership-by-geometry](../core/output-membership-by-geometry.md):
frame callbacks, `wl_surface.enter` and foreign-toplevel `output_of_window`
still decide "which output" by bbox overlap. None draws or delivers input
to the wrong place.

The "never over it" wording is now true across outputs as well and says so
(`docs/protocols.md`, the `docs/ipc.md` `rect`/`fullscreen` rows, the
`WindowSnapshot` docs); the unsourced "niri answers the same way" was
dropped from `docs/protocols.md` and the `above_windows` doc.

## Evidence

Recorded in PR #224's description (exact commands, SHAs, raw numbers), with
the artifacts on the dev VM: live repro script `/tmp/bleed-repro.sh`, before
(`a02cea9`) `/tmp/bleed-before/`, after (`a9d1b60`) `/tmp/bleed-after/`;
fail-first `/tmp/bleed-failfirst-a02cea9.txt`; single-output byte identity
`/tmp/bleed-byteid/`; benchmarks `/tmp/bleed-bench-{before,after}.txt`;
gate `/tmp/bleed-gate-a9d1b60.log`. Headline: output 1's screenshots go
from showing output 2's window (with ring) across x 525-800 to pure
background, and a click there stops focusing it; a two-output frame is
~8% cheaper (the bleed is no longer composited); the per-motion hit test
costs +19 ns on a window, nothing on background.

---

The original entry follows.

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
