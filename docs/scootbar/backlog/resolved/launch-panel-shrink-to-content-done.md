---
title: "scootlaunch: shrink the panel to its content"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-05"
---

# scootlaunch: shrink the panel to its content: RESOLVED

Resolved 2026-11-05 on `feat/scootlaunch-shrink-panel`. What landed is the
"What to do" list below, as built; evidence is in the PR.

Filed 2026-10-19. Serves **daily-drive**: it is visual completeness of a
launcher people open many times a day. It has no agent-facing surface (`--list`
and `--query` are untouched), so it does not serve computer use.

## The gap

Found reviewing #674 (2026-10-19). The panel is as tall as its `rows` (10 by
default) whatever matches: with the three apps of the test fixture it is 281
logical pixels tall and 120 of them hold anything, so 57% of the panel is empty
fill (measured from the headless screenshots, 360..640 against 360..479 on a
1600x1000 output).

## What to do

Size the panel to its rows: one row per match, at least `MIN_ROWS` (3: the
field, a row and the one-line "No applications" / spawn-error note with room
around them, and not a bare field when the list is empty), at most `rows`
(`min(3, rows)` when `rows` is smaller).

- **No resize.** The surface keeps the height of all `rows`; only the drawn
  panel shrinks, pinned to the top. A resize would need a `set_size`,
  a configure round trip and a new shm buffer (the pool is keyed on size) on
  a key, and a compositor-centered surface would move the search field as the
  list changes. The buffer, the stage layers and the layer-shell state do not
  change, so the key path allocates what it did.
- **Vertical position.** Pinning the top keeps the field still, and a short
  panel then sits above the middle of the output: the optional
  "slightly-above-center" falls out of the design, with no config key.
- **One height, two meanings, named.** `App::height` stays the surface's full
  panel height (`surface_size`, the shadow's room fit); `App::shown_height` is
  the drawn panel (the pointer's panel test, the input region). The input
  region is the panel's rectangle whenever the panel is short, so a press below
  it is outside and dismisses.
- **One layout.** The rows are fitted once for the full height; a short panel
  is that layout's height for fewer rows, so `row_h` never changes with the
  count (no jitter at a fractional scale, hit tests stay a division).
- The stage's shadow table is keyed on the radius the panel can draw, not on
  its size, so a changing row count rebuilds nothing (the first version
  allocated 17 times over 18 key frames instead of 9; a test pins it).
- Typing only narrows the list, so the first frame is the tallest: every pooled
  layer is already at its largest.

## Not in this ticket

- `--dmenu` keeps its fixed `--lines` height: that is an explicit request, its
  panel paints through a separate path, and a script that pipes few lines would
  want a fixed layout anyway.
- A config key for the minimum or the position.
- The entrance and exit scale about the surface's center, which is no longer
  the center of a short panel: the panel's center moves about 3 px over the
  160 ms of a 3-row entrance. Not worth a separate pivot.

## Docs (same PR)

Done: a new Size section and the `rows` and click-outside entries in `site/src/content/docs/scootlaunch/panel.md`; the two docs screenshots regenerated (`SCOOTLAUNCH_SHOTS=<dir> cargo nextest run -p scootlaunch icons_docs`).
