---
title: "Popups that match rounded windows: corners and a border"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-04"
---

# Popups that match rounded windows: corners and a border

Filed 2026-10-03, from the same showcase shoot. Serves **daily-drive**
(looks): the volume and WiFi popups are square with a 1 px border, next to
scoot windows rounded at 14 px and a rounded or edge-to-edge bar; in a
light theme they read as plain boxes. scoot's own docs leave popup
rounding open ("whether menus round too is a separate decision",
`docs/configuration.md` `corner_radius`).

## What to do

- A `[bar] popup-radius` (or reuse a shared radius), drawn by scootbar
  itself in the popup's own buffer (the popup surface is the bar's client
  surface, so the compositor's rounding does not apply), with the corners
  transparent.
- Measure the cost per popup frame (the popup draws only while open).

## Not in this ticket

Drop shadows (they need a larger surface than the content, or compositor
support) and blur.

## What landed (2026-10-04, PR #429)

`bar.popup-radius`, unset following the bar's `radius`, drawn by scootbar
in the popup's own buffer with transparent corners; the border follows the
arc at one logical pixel at every scale and the rows are clipped to the
inside arc. Corner tables are built once per open popup (no per-frame
allocation; the warm-popup test paints both shapes with zero allocations),
buffers are `ARGB8888` only while rounded, the compositor gets the opaque
region and the rounded input shape. Tooltips round the same way (same
paint path). `music-desk` sets `popup-radius = 14` to match its windows;
`radial-burst` (16) and `vinyl-sunset` (12) follow their bars.

Cost: one popup repaint 14,948 ns square vs 25,615 ns rounded (262x162,
release); open+close 548.8 us vs 573.1 us, 4 wakeups each, idle level;
`.text` +4,736 B. Screenshots of volume, network and power in all three
looks, before and after, in the PR. The tray menu has none: no dbusmenu
peer tooling on the bench box (its paint path is the same one, covered by
the unit tests). Full evidence in the PR body and `lightest.md`'s
`M6 popup corners` section.
