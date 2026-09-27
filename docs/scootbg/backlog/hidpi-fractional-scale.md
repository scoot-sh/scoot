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
- Today `wl_surface.preferred_buffer_scale`/`_transform` are ignored and
  `query`'s `logical` is an integer-scale estimate until the surface is
  configured ([ticket 3](resolved/outputs-and-layer-surfaces-done.md#for-the-next-tickets)).
- A scale change rescales from the source (re-decoding if it was dropped),
  never from the previous scaled buffer.

Check on scoot with a fractional `scale` in its config: the screenshot must
be pixel-sharp, not a compositor-upscaled blur.

**From [ticket 6](resolved/images-decode-and-fit-done.md#for-the-next-tickets):**
an image is drawn at the configured surface size times `wl_output`'s
integer scale, with `set_buffer_scale`, like a full-size color. At a
fractional scale that is larger than the output (1.5 on 1600×1000: a
2134×1334 buffer for a 1067×667 surface) and the compositor scales it
down, so it is sharp but not device-exact, and costs ~78% more pixels to
draw and hold than the output has. A new scale re-renders from the file
only when the buffer size changes (scale 2 with the mode doubled keeps
it, and attaches it again: Smithay-based compositors read a new
`set_buffer_scale` only with an attach, see
[the compositor item](../../backlog/core/buffer-scale-needs-a-new-buffer.md)).
The size is decided in one place, `daemon::change::image_dims`, and the
buffer is always the worker's full-size render, so drawing at a
`wp_fractional_scale_v1` size is a change there plus a viewport
destination.
