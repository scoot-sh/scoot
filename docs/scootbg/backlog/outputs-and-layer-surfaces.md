---
title: "One background layer surface per output, across hotplug"
status: "open"
area: "scootbg"
priority: "high"
blocked: null
---

# One background layer surface per output, across hotplug

- Track `wl_output`s (name, description, mode, scale) as they appear and
  vanish; bind `xdg-output` only if `wl_output` v4's `name` is missing.
- Per output: a `zwlr_layer_surface_v1` on the `background` layer,
  namespace `wallpaper`, anchored to all four edges, exclusive zone `-1`,
  keyboard interactivity `none`, and an empty input region so pointer
  events fall through.
- Handle `configure` sizes of 0 (use the output's size), a `closed` event
  (the compositor removed the surface: recreate once, then give up loudly),
  and an output removed while an image is still decoding for it (drop the
  result, do not panic).
- Zero outputs is a normal state (a headless session before its first
  output, a laptop with the lid shut): the daemon idles and waits.

Test on scoot `--headless --outputs 2` (both outputs covered, the second
checked by `scootctl screenshot --output 2`), and on at least one other
layer-shell compositor before calling it portable.
