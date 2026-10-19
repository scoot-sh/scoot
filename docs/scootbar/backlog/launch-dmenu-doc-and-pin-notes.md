---
title: "scootlaunch dmenu: document three edge behaviors, pin color overrides"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
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
