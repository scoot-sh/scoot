---
title: "scootlaunch fit: the remainder is dead space at the bottom, scale_len and scaled_length disagree, and a 3x entrance frame outruns 16.6 ms"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
---

# scootlaunch fit: the remainder is dead space at the bottom, scale_len and scaled_length disagree, and a 3x entrance frame outruns 16.6 ms

Filed 2026-10-31, from the review of #839
([launch-ignores-preferred-scale](resolved/launch-ignores-preferred-scale-done.md)).
Serves **daily-drive**: a launcher that is a few pixels off at a fractional
scale is the soft, uneven look the scale work set out to remove.

## The gap

Three small findings, none a crash and none a blur.

1. **The fit leaves its remainder at the bottom.** `fit_row`
   (`crates/scootlaunch/src/layout.rs`) divides the room left after the
   padding and gap by `rows + 1` with floor division, so up to `rows` device
   pixels are dead space under the last row: 8 px for the apps panel at 1.25x,
   9 px for dmenu at 1.75x with 12 rows. The `Layout::fitted` doc and the
   #839 commit message say a row gives up "at most 2 px", which overclaims:
   the per-row loss is bounded, but a font at 8 rows with a remainder of 0
   still measured 3 px.
2. **Two rounding rules for one length.** `scale_len`
   (`crates/scootlaunch/src/panel.rs`, `f64::round`) and
   `scootui::density::scaled_length` (integer arithmetic on the v120 scale)
   differ by 1 px for about 39 non-dyadic v120 values: 122, 123, 130, 131,
   138, 145, 146, 153, 161, 183, 191, 206, 244 to 247 and 260 to 262. The
   buffer is sized by one and the compositor's logical-to-buffer mapping by
   the other.
3. **A 3x entrance frame is probably over budget.** At 17 ns a destination
   pixel
   ([launcher-scaled-composite-cost](launcher-scaled-composite-cost.md)) a
   3x launcher entrance frame (about 1240x940) is about 20 ms, over the
   16.6 ms of a 60 Hz frame. Not measured; the note is on that entry.

## What to do

1. Add `rem / 2` to the top and bottom padding in `fitted` and
   `dmenu_fitted`, keeping every row the same height so `row_at` stays a
   division. Correct the "at most 2 px" wording to what the code does.
2. Replace `scale_len` with `scaled_length` so the buffer and the
   compositor's mapping round alike; pin it with a test over every v120
   value from 30 to 1920.
3. Measure the 3x entrance frame with `bench_effects` and record it on
   `launcher-scaled-composite-cost`.

## Not in this ticket

Changing the panel's geometry at 1x or at the integer scales.
