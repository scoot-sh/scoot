---
title: "Drawing at real device pixels on scaled outputs"
status: "open"
area: "scootbg"
priority: "high"
blocked: null
---

# Drawing at real device pixels on scaled outputs

- With `wp_fractional_scale_v1` and `wp_viewporter`: allocate the buffer at
  `ceil(logical × scale)` device pixels, set the viewport destination to the
  logical size, and redraw on `preferred_scale` changes.
- Without fractional scale: `wl_surface.set_buffer_scale` with the integer
  `preferred_buffer_scale` (surface v6) or the output's scale.
- A scale change rescales from the source (re-decoding if it was dropped),
  never from the previous scaled buffer.

Check on scoot with a fractional `scale` in its config: the screenshot must
be pixel-sharp, not a compositor-upscaled blur.
