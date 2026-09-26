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
- Pace with frame callbacks so a hidden output (a fullscreen window over
  it, where the compositor stops sending callbacks) costs nothing.
- `--no-animate` to show the first frame only.
