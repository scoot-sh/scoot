---
title: "scootlaunch ignores the compositor's preferred scale: every mode draws at 1x on a HiDPI output"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
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
wiring is missing. `site/src/content/docs/scootlaunch/panel.md` says "a 1.5
output draws a 1.5x bitmap", which is not what happens live.

## What to do

Feed the three events into `app.demand` (as scootbar does), redraw on a
change, and add a live test that reads `create_buffer` off `WAYLAND_DEBUG`
at `output-scale 2`. Check the pointer maths (`panel_point`, `hit_row`) at a
fractional scale, and the shadow room and input region with a look that has
a shadow.

## Not in this ticket

Changing the panel's geometry at 1x.
