---
title: "Withhold a top-layer surface's frame callbacks under any covering fullscreen window, not only an opaque one"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-05"
---

# Withhold a `top` surface's frame callbacks under any covering fullscreen window

Filed 2026-11-05 from the [`notify-fullscreen-hold-spike`](../../scootbar/backlog/resolved/notify-fullscreen-hold-spike-done.md)
findings. Serves **daily-drive**: it is the precondition for a "hold cards while fullscreen"
mode in scootnotify, and it removes a wakeup per frame for any animated `top` client.

## The gap

`withhold_frame` (`crates/scoot/src/compositor/layer_shell/occlusion.rs`, the `Layer::Top` arm)
withholds a `top` surface's callbacks only when `covered_by_fullscreen(output)` **and**
`covering_window_is_opaque`. But a `top` surface is not drawn at all while any fullscreen window
covers its output, opaque or not (`layer_shell::above_windows`, used by
`render/elements.rs:433`). Measured on v0.7.9, headless, CPU renderer (screenshots from the spike):

- fullscreen window with an alpha-format buffer and no opaque region, or at alpha 0.5: the `top`
  surface is **absent from the screenshot** and its callback **was served** 18 ms after commit;
- the same window with an opaque format or region: absent and **never served** while covered.

The opacity guard is the rule for `bottom`/`background`, which a translucent window lets show
through. It does not apply to `top`. The commit that added it (`732aaf1af`) states no reason for
it on `top`, and no test pins `top` being served under a translucent cover
(`compositor/fullscreen/tests/occlusion.rs`).

Effect today: a client that starts a timer at its first frame callback (scootnotify's expiry
clock) starts it while the surface is invisible.

## What to do

Make the `Layer::Top` arm `state.covered_by_fullscreen(output)`. Pin with tests: a translucent
fullscreen window withholds `top`, still serves `overlay`, and the callback completes on the first
frame after the fullscreen window leaves (the opaque case already has this in
`the_callback_resumes_when_the_cover_lifts`). A frame-callback path change, so
re-run the occlusion tests and the render benchmark.

## Not in this ticket

The scootnotify mode itself. See the resolution of the spike entry.

## Resolution

The `Layer::Top` arm of `withhold_frame` is now `state.covered_by_fullscreen(output)`; the
opacity guard stays for `bottom`/`background` (`covering_window_is_opaque` is still used by
`opaque_window_spans_output`), and `overlay` is never withheld. Pinned in
`compositor/fullscreen/tests/occlusion.rs`: an alpha-format cover (no opaque region) and a
half-alpha `wp_alpha_modifier_v1` cover both withhold `top`, still serve `overlay`, and the held
callback completes on the first frame after the cover lifts.
