---
title: "scootlaunch dmenu: document three edge behaviors, pin color overrides"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-04"
---

# scootlaunch dmenu: document three edge behaviors, pin color overrides

Filed 2026-10-19. Serves **daily-drive** / **computer use** (pick one, say why).

## The gap

What is wrong or missing, with evidence (file paths, measured numbers).

## What to do

The proposed shape, and the edge cases to pin.

## Not in this ticket

What is deliberately out of scope.

Found reviewing #678 (2026-10-19), all non-blocking.

- `cli.md`: say `--prompt-only` with `--no-run-if-empty` always exits 1 without opening.
- `cli.md` Run mode: a second `--run` dismisses the first through the shared toggle and exits 0 without launching.
- Pin `apply_colors` end to end: a headless `--background-color=… --dmenu` screenshot assert.
- Decide whether `--dmenu --password` with a visible list should stop filtering on the secret (a matching-rows side channel; the WiFi call site uses `--prompt-only`, so unaffected).

## Docs (same PR)

The first two items are docs; update `site/src/content/docs/scootlaunch/cli.md`.

## Resolution

Landed 2026-11-04. The two edge behaviors are in `cli.md` (checked against
`cmd_dmenu` and `panel::open`: a second `--run` gets `Open::Signalled` and
returns 0). `tests/dmenu.rs::color_flags_reach_the_drawn_pixels` pins
`apply_colors` end to end, with a no-flag control. The password question was
real: `--dmenu --password` with a stdin list ranked, highlighted and selected
on the secret (and Enter printed the matching line, not the secret), and
`rerank` folded the secret into two unwiped scratch buffers even under
`--prompt-only`. Password mode is now list-less and never reads the entry in
`rerank`; unit and screenshot tests pin it.
