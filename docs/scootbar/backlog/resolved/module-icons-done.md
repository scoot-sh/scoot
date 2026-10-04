---
title: "Icons on every module that shows a value: battery, brightness, bluetooth, media"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-03"
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

## Resolution (2026-10-04, PR #411)

Landed as `feat(scootbar): icons on battery, brightness, bluetooth, media
and window-title`, on the shape `network-signal-icon` establishes (one
glyph, or one glyph per level picked by the value; the same loud
validation naming the dotted key):

- **battery**: `icon` takes one glyph or five (empty to full, quintiles
  0–19 … 80–100, `level_for`), plus `icon-charging` and `icon-full`
  (full wins over charging, both over the level); a static
  `icon-path`/`icon-image` beside the array is refused as two icons, and
  `show-text = false` draws only the icon (the tooltip already names the
  level: `Discharging 72%`).
- **brightness**: `icon` takes one glyph or four (dim to bright,
  quartiles), plus a static path/picture and `show-text`.
- **bluetooth**: `icon-off`, `icon-on`, `icon-connected` per state over a
  static icon, plus `show-text` (every tooltip already names the state).
- **media**: `icon-playing`, `icon-paused` over a static icon, replacing
  the built-in vectors; plus `show-text`. No `icon-stopped`: a stopped
  player shows nothing, so there is nothing to draw it beside.
- **window-title**: one static `icon`, drawn only when a window is
  focused (never for the placeholder; with `show-text = false` and no
  window the module hides, as with an empty placeholder), plus
  `show-text` (the tooltip already carries the full title).
- **workspaces**: no icon, decided. Its numbers and pill are the content
  — a leading static glyph would decorate without informing — and
  per-workspace icons would need the compositor to name one, which no
  protocol carries (`ext-workspace-v1` has no icon field).
- `cli.md` has one shared "Icons" paragraph now (the per-state and
  per-level table, the `show-text` rule, a Nerd Font example with every
  module's codepoints, each verified against Pictogrammers/MDI and
  covered by the Asahi box's Nerd Font); the module sections point to it.
  `scootbar daemon --help` names the new keys per module.

Evidence: new tests fail-before proven on the Asahi M2 (six sabotage
cases FAIL — level boundaries, per-state precedence, the icon gate, the
config wiring — and all pass restored); `nextest -p scootbar` 1272
passed on the dev VM (7 failed: 6 popup/tooltip integration failures
from the VM's old scoot binary, failing identically on main, and 1
snapshot failure from AppleDouble `._` files my first tar shipped,
green after cleanup) and 1278 passed plus the snapshot test on the
Asahi M2 with this tree's own scoot; `cargo test -p scootbar` green on
both; clippy matrix clean except two pre-existing sparse-combo
dead-code failures failing identically on main (`brightness+clock`'s
`Input.at`, `window-title+clock`'s `ArgKind` variants); `fmt --check`
and `scripts/backlog check` clean (the 3 check problems pre-date this).
Live on the Asahi M2 (real battery at 100% Full, backlight 107/509,
wlan0 at −54 dBm, bluetooth on, audio at 49%): `micons-a-top.png`
(every module placed and iconed, light palette), `micons-b-top.png`
(`show-text = false` on brightness and bluetooth),
`micons-c-top.png` (the `apple-panel-bl: 21%` tooltip on the icon-only
brightness). No media player was running on the box, so media hides
there; none was started. Ratchet (release, Asahi): `.text` 1,609,032 →
1,622,344 (+13,312 B, +0.8%), file size unchanged at 2,101,984 B both;
idle with the modules placed 5776 kB RSS and 12–16 wakeups/20 s before
and after (the iconed bar is 6368 kB: +592 kB is the Nerd Font fallback
file, the documented cost of any fallback font).
