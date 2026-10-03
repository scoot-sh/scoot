---
title: "Usable area is one pixel narrower than the output after a scale reload"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-03"
---

# Usable area is one pixel narrower than the output after a scale reload

Filed 2026-09-29. Found while testing `[[outputs]]` (`Asahi.md` Test 16),
but it predates that work. Serves **computer use**: an agent that places or
checks windows against `scootctl outputs`' `usable` gets a rectangle one
logical pixel short of the one a fresh session reports, and the arrangement
is sized to that.

## The gap

Take an output whose logical width is fractional at the new scale, and
reload the scale onto it. Its `usable` then comes out one pixel narrower
than its `rect`, with no bar mapped. A session started at that scale
reports the two equal.

Reproduced on `main` at `39ca4c05f` (debug build on the Asahi box, a
`--headless --width 1280 --height 720` session):

- Started at `[output] scale = 1.5`: `rect` 854x480, `usable` 854x480.
- Started at 1.0, then reloaded to 1.5: `outputs.scale` applies, `rect`
  854x480, `usable` **853**x480.

It is reachable on hardware: DP-1 at 1280x720, reloaded from 1.0 to 1.5
through its `[[outputs]]` entry, read `rect` 854 and `usable` 853 (Test
16).

The likely cause is two roundings meeting. `rescale_outputs` files the
`Space`'s `ceil` geometry with the core (`OutputChanged`). The
re-clamp and the `refresh_layer_zone` that follow then take the layer
map's non-exclusive zone, which Smithay may compute with another rounding
of `physical / scale`. At startup, `OutputAdded` files the `ceil` area
before any zone exists. This is a hypothesis, not yet read in full against
the pinned Smithay's `LayerMap::arrange`.

## What to do

- Read which rounding the layer map's zone uses at the pinned rev.
- Make the reload path file what startup files: equal to `rect` with no
  exclusive zone, at every fractional scale.
- Pin it in a test: a start at 1.0 reloaded to 1.5 on a 1280-wide output
  must match a start at 1.5.

## Not in this ticket

Exclusive zones that legitimately shrink `usable`.

## Resolution (2026-10-03, PR #395)

Landed as `fix(scoot): file the output geometry as usable when no layer reserves space` (`8ac76acc`). Root cause confirmed against pinned Smithay fork `035d447c`: `Space::output_geometry` uses `to_i32_ceil` (854) while `LayerMap` uses `to_i32_round` (853); reload filed `OutputChanged` correctly then overwrote it with the rounded zone. Fix files the Space geometry when no mapped layer has an effective exclusive zone. Pinned by `reload::tests::a_fractional_scale_reload_files_the_startup_usable_area` (fails on base with 853 vs 854) plus a `reserves_space` matrix; 172-test compositor subset green, clippy/fmt clean, CI green.
