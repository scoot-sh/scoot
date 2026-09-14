---
item: "4b"
title: "Window decorations"
status: "done"
area: "rendering"
pr: 7
commit: "21b9c3e"
---

# Window decorations

**4b decorations — DONE, merged to `main` at `21b9c3e`, PR #7.**
`crates/flexwm/src/compositor/decorations.rs`: niri-style focus ring
(width clamped to at most half the gap at load time) drawn in the
layout's own gap + a background color (`render_output`'s `clear_color`,
not an element — bottom-most by construction) + `zxdg_decoration_manager_v1`
answering `ServerSide` when `prefer_no_csd` (default true).
`headless.rs::render()` moved off `space::render_output` to
`space_render_elements`+`OutputDamageTracker::render_output` directly so
windows draw on top of the ring (element order is back-to-front,
`.iter().rev()`). Ring buffers are persistent per-window (`SolidColorBuffer`,
stable `Id`, updated not rebuilt). Verified via real pixel-sampled IPC
screenshots across all three backends including real `--tty` hardware.
`imagemagick` added to `vm/configuration.nix` for pixel-reading.
