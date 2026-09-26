---
title: "Animated wallpapers: GIF, APNG, animated WebP (milestone 2)"
status: "open"
area: "scootbg"
priority: "medium"
blocked: "reuses the frame pacing built for transitions.md"
---

# Animated wallpapers: GIF, APNG, animated WebP (milestone 2)

- Decode frames once, scale once per output; keep them compressed (or as
  diffs) under a memory cap, and refuse or downscale animations past it.
- Pace with frame callbacks. That only makes a covered wallpaper free if
  the compositor withholds callbacks from surfaces nobody can see, and
  **scoot does not today**: it sends `frame` to every mapped layer surface
  on each rendered frame, with no occlusion check (`headless.rs`, the
  post-render path). A fullscreen video over an animated wallpaper would
  keep scootbg blending at the video's frame rate. So this item needs
  [the scoot-side fix](../../backlog/core/frame-callbacks-for-hidden-surfaces.md)
  first, or its own pause (e.g. `ext-foreign-toplevel` fullscreen state),
  measured either way.
- `--no-animate` to show the first frame only.
