---
title: "scootlaunch fit: the remainder is dead space at the bottom, scale_len and scaled_length disagree, and a 3x entrance frame outruns 16.6 ms"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-31"
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
   the per-row loss is bounded, but a font size of 8 with zero list rows
   still measured 3 px.
2. **Two rounding rules for one length.** `scale_len`
   (`crates/scootlaunch/src/panel.rs`, `f64::round`) and
   `scootui::density::scaled_length` (integer arithmetic on the v120 scale)
   differ by 1 px for 25 v120 values up to 300 (the 19 below 263: 122, 123,
   130, 131, 138, 145, 146, 153, 161, 183, 191, 206, 244 to 247 and 260 to
   262; plus 275 to 277 and 290 to 292), and for about 180 values up to 16x
   at lengths up to 1500. The
   buffer itself is sized by `scaled_length` (through `Demand::buffer_dims`)
   while the panel's device size inside it comes from `scale_len`, clamped
   to the buffer with `.min(buffer_w)`: one rule would do.
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

## Resolution

Landed 2026-11-01.

1. `Layout` carries the fit remainder as `extra`, split top/bottom
   (`top_extra`/`bottom_extra`, within 1 px), so the panel height lands on
   the scaled height: apps 1.25x (10 rows) went from 8 px of dead space to
   top 4 / bottom 4; dmenu 1.75x (12 rows) from 9 px to top 4 / bottom 5.
   Rows stay uniform, so `row_at` stays a division (the click-coverage test
   is unchanged and green). Layouts that already fit keep `extra` 0, so 1x
   and the integer scales draw what they drew. The "at most 2 px" wording
   is corrected to what the code does (3 px measured at font 8 with no
   rows) in the `fitted` doc, the scale test, and the #839 resolution note.
2. `scale_len` now takes the `Scale` and rounds the integer way the buffer
   does (`scaled_length`; exact multiply at integer scales), so the panel
   inside matches the buffer and the compositor's mapping. The sweep test
   (`scale_len_matches_scaled_length_at_every_v120_and_length`: v120
   120..1920 by lengths 1..1500) passes; the old `f64` rule is pinned to
   fail it (`the_old_f64_scale_len_disagrees_with_scaled_length`: the 25
   v120 values up to 300 each have a 1 px witness). The 1x byte pin is
   unchanged (`0x6226_31d1_2e84_8874`); the scaled pin moves
   (`0xa352_c849_814f_5f69` to `0x219f_0ed0_9338_2d41`) for the fractional
   half alone — the integer-2x half draws the same bytes (exact multiply,
   `extra` 0 at the pinned looks), the rows shift by the top remainder and
   1 px widths/heights move where the old round disagreed.
3. 3x entrance frame measured with `bench_effects` (release, `taskset -c 3`,
   three runs): median 19.3 to 19.5 ms (p5 3.9 to 4.0 ms) at 1242x942 —
   over the 16.6 ms budget, as extrapolated. Recorded on
   `launcher-scaled-composite-cost`; the cheapest fix stays the filed
   row-blend (no output change), not implemented here.
4. `site/.../scootlaunch/panel.md` font symptom now states the real order
   (given `font`, then `font-file`, family, well-known), matching
   `troubleshooting.md`.
