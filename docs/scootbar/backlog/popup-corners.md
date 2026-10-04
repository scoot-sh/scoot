---
title: "Popups that match rounded windows: corners and a border"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M6"
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
