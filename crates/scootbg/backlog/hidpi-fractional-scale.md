---
title: "Drawing at real device pixels on scaled outputs"
status: "open"
area: "scootbg"
priority: "high"
blocked: null
---

# Drawing at real device pixels on scaled outputs

- With `wp_fractional_scale_v1` and `wp_viewporter`: allocate the buffer
  at the logical size times the scale, rounded halfway away from zero as
  the protocol specifies, set the viewport destination to the logical
  size, and redraw on `preferred_scale` changes. For a surface covering
  the whole output, that can land a pixel off the output's real mode
  (2560 px at 1.5 is 1707 logical, which rounds back to 2561): prefer the
  output's current mode size there, and test the rounding cases.
- Without fractional scale: `wl_surface.set_buffer_scale` with the integer
  `preferred_buffer_scale` (surface v6) or the output's scale.
- A scale change rescales from the source (re-decoding if it was dropped),
  never from the previous scaled buffer.

Check on scoot with a fractional `scale` in its config: the screenshot must
be pixel-sharp, not a compositor-upscaled blur.
