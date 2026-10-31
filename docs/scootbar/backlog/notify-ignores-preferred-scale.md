---
title: "scootnotify ignores the compositor's preferred scale: cards draw a 1x buffer upscaled on a HiDPI output"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
---

# scootnotify ignores the compositor's preferred scale: cards draw a 1x buffer upscaled on a HiDPI output

Filed 2026-10-31, left over from
[launch-ignores-preferred-scale](resolved/launch-ignores-preferred-scale-done.md).
Serves **daily-drive**: notifications are the other overlay a HiDPI laptop
shows all day, and their text and border are soft.

## The gap

`crates/scootnotify/src/popup.rs` binds neither `wp_viewporter` nor
`wp_fractional_scale_manager_v1` (`Globals { viewporter: None,
fractional_scale: None }`), and `wl_surface`, `wl_output` and
`wp_fractional_scale_v1` are `empty_dispatch!`, so `Demand::scale()` stays 1:
every card draws into a buffer the size of the stack's logical size and the
compositor upscales it. The site says so (`scootnotify/index.md`: "Popups
draw at scale 1 in v1").

## What to do

Feed `wl_surface.preferred_buffer_scale`, `wp_fractional_scale_v1`'s
`preferred_scale` (bound only with a `wp_viewporter`) and the `wl_output.scale`
of the outputs the surface entered to the `Demand`, as the bar and the
launcher do, and redraw when the scale to draw at changes. The layout, the
input region, the hit test and the motion geometry stay in logical pixels
(the region is in surface-local logical pixels whatever the buffer is); only
the paint and the motion layers are in device pixels, with every rectangle
mapped by its edges so the stack's far edges land on the buffer's. The
buffer is exactly `Scale::buffer(logical)`.

Share the launcher's `OutputScales` through scootui instead of copying it.

## Not in this ticket

Changing the cards' look at 1x or at an integer scale (the 1x byte
fingerprints stay as they are); icons sharper than the 96 px they decode to
(exact at 2x).
