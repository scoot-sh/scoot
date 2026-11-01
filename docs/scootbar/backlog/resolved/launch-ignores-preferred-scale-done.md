---
title: "scootlaunch ignores the compositor's preferred scale: every mode draws at 1x on a HiDPI output"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-31"
---

# scootlaunch ignores the compositor's preferred scale: every mode draws at 1x on a HiDPI output

Filed 2026-10-31, found building the dmenu depth and motion follow-up
(#807). Serves **daily-drive**: a HiDPI laptop gets a launcher the compositor
upscales, so its text and border are soft.

## The gap

`crates/scootlaunch/src/panel.rs` handles no scale event: `wl_surface`,
`wl_output` and `wp_fractional_scale_v1` are all `empty_dispatch!`, so
`Demand::prefer_fractional`, `prefer_buffer_scale` and `set_output_scale`
(which scootbar calls from `daemon/surfaces.rs`) are never reached and
`Demand::scale()` stays 1. Every mode (apps, run, dmenu) draws a 1x buffer
and the compositor scales it up.

Measured 2026-10-31 on the dev VM, headless `scoot` with
`scoot msg output-scale 1 2`, `WAYLAND_DEBUG=1`, `scootlaunch --run`:

```
<- wl_surface@13.preferred_buffer_scale, (2)
<- wp_fractional_scale_v1@15.preferred_scale, (240)
-> wl_shm_pool@17.create_buffer(wl_buffer@18, 0, 520, 282, 2080, 0)
-> wp_viewport@14.set_destination(520, 282)
```

A 2x output should get a 1040x564 buffer. The device-scale painters and
their tests exist (`paint_panel` at `Scale::Fractional`), so only the event
wiring is missing. `site/src/content/docs/scootlaunch/panel.md` says the
launcher draws at 1x; restore the icon-scale and output-scale sentences when
this lands.

dmenu at 1.5x: the device layout is up to 3 px taller than
round(logical x 1.5); derive the buffer height from the device layout. The
live test `under_a_shadow_a_press_on_it_dismisses_and_a_row_click_still_picks`
hardcodes the 1x row geometry and needs updating then.

## What to do

Feed the three events into `app.demand` (as scootbar does), redraw on a
change, and add a live test that reads `create_buffer` off `WAYLAND_DEBUG`
at `output-scale 2`. Check the pointer maths (`panel_point`, `hit_row`) at a
fractional scale, and the shadow room and input region with a look that has
a shadow.

## Not in this ticket

Changing the panel's geometry at 1x.

## Resolution

Landed 2026-10-31. scootlaunch feeds the compositor's three scale hints to
`app.demand` and redraws when the scale to draw at changes:
`wl_surface.preferred_buffer_scale` (v6),
`wp_fractional_scale_v1.preferred_scale` and the `wl_output.scale` of the
outputs the surface has entered (`panel/scale.rs`). A buffer is now exactly
the surface's logical size times the scale, at every scale and in every
mode; at `output-scale 2` the ticket's trace gives a 1040x564 buffer under a
520x282 destination (`tests/scale.rs`). The fraction is only followed with a
`wp_viewporter`, as the bar does.

The dmenu mismatch was wider than the ticket's 3 px: the device layout
rounds each measurement on its own, so it stood up to 11 px over the scaled
logical height (a tall list at 1.25x). Growing the buffer to hold it would
make the viewport resample the whole panel (3% at worst), which is the blur
this fixes, so the layout is held to the scaled height instead
(`Layout::fitted`): the rows give up a few px (3 px measured at font 8
with no rows), never below the text's own height, and nothing changes
at 1x or at the integer scales. Clicks use
the drawn layout at the real scale factor (`row_at`), taking the pointer at
the middle of its logical pixel.

`under_a_shadow_a_press_on_it_dismisses_and_a_row_click_still_picks` stays
the 1x case; `a_scaled_dmenu_under_a_shadow_fills_its_buffer_and_still_picks`
is the same at 1.25x to 2x with the geometry derived from the scale.

Left alone, as filed: scootnotify has the same `empty_dispatch!` gap for the
scale events (it binds neither `wp_viewporter` nor the fractional-scale
manager), and the cost of a scaled composite is
[launcher-scaled-composite-cost](../launcher-scaled-composite-cost.md), now
paid on every open at 2x (an entrance frame is 8.9 ms there).
