---
title: "scootlaunch icon-less entries share one cached tile letter (Printer drew C)"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
---

# scootlaunch icon-less entries share one cached tile letter (Printer drew C)

Filed 2026-10-21. Serves **daily-drive**: a wrong tile letter sends the
eye to the wrong row in the launcher.

## The gap

`crates/scootlaunch/src/icons.rs` (`Cache::get`) keys the icon cache by
`(icon-name, side)` only and falls back to `Icon::Tile(tile_letter(label))`
on a miss. Every entry with no `Icon=` therefore shares the single key
`("", side)`: whichever icon-less row draws first caches its letter and
every later icon-less row reuses that tile. Evidence: the full-look
rig's take 2 (PR #707) showed a synthetic `Printer` entry drawing a `C`
tile (`tile_letter("Printer")` is `P`; per-row code passes the row's own
label, so the panel asked for `P` and the cache answered `C`,
presumably Calculator's). Any menu with two icon-less entries reproduces
it; only the first is correct.

## What to do

Key tile fallbacks by label as well (or don't cache `Tile` values at
all: they cost nothing to redraw), with a regression test opening two
icon-less entries and asserting distinct tiles. Pin: the `MAX_ENTRIES` /
`MAX_BYTES` eviction accounting counts tiles as zero bytes, so a
no-cache-tiles change must keep the bounds intact.

## Not in this ticket

The README hero workaround (PR #707 gives every synthetic entry a
distinct `Icon` key, so no two visible rows share `("", side)`).
Resolving more real icons from the theme.
