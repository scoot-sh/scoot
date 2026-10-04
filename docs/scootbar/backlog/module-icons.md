---
title: "Icons on every module that shows a value: battery, brightness, bluetooth, media"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "M6"
---

# Icons on every module that shows a value: battery, brightness, bluetooth, media

Filed 2026-10-03. Serves **daily-drive**: the maintainer, sharing their own
scoot desktop (a light theme, a floating bar, an icon beside every value):
"Despite us creating low resource usage, this can all still be beautiful."

## The gap

Measured with `scootbar daemon --check` on `main` (`3922c879d`): `icon`
is accepted by `clock`, `network`, `volume` and `microphone`, and refused
as an unknown field by `battery`, `brightness`, `bluetooth`, `media`,
`window-title` and `workspaces`. On a real bar (the Asahi M2 showcase run,
2026-10-03) that leaves `on`, `21%` and `100%` as bare text beside iconed
WiFi and volume, which reads as unfinished.

## What to do

Use the shape [network-signal-icon](resolved/network-signal-icon-done.md) establishes
(a key that takes one glyph, or one glyph per level, picked by the value),
so every module's icon keys look the same:

- **battery**: one glyph per level (five, empty to full, by percent) plus
  `icon-charging` (and `icon-full` when plugged in at 100%), since the
  charge state matters more than the exact level.
- **brightness**: one glyph, or one per level (three or four).
- **bluetooth**: one glyph per state (`icon-off`, `icon-on`,
  `icon-connected`), as network has per state.
- **media**: one glyph per state (playing, paused, stopped).
- **window-title**: one static glyph (optional; most bars show none).
- `icon-path` and `icon-image` where the module's siblings take them, and
  `show-text = false` where an icon can stand alone (brightness and
  bluetooth especially), with the text moved into the tooltip as network does.
- Validation as the existing icon keys do it: loud, naming the key.
- `cli.md`: one shared "Icons" paragraph the modules point to, rather than
  each module restating the rules; an example config with Nerd Font
  codepoints for every module.

Done when a bar with every module placed and iconed screenshots
cleanly (the Asahi M2 has real battery, backlight, WiFi and bluetooth),
and the ratchet's size and idle rows are measured.

## Not in this ticket

Icon themes (`IconName` lookups, as the tray needs:
[tray-icon-themes](tray-icon-themes.md) once #405 lands), and colored or
per-state icon colors (a later theming entry if the maintainer wants one).
