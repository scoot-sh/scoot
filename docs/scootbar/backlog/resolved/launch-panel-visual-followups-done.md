---
title: "scootlaunch panel visual follow-ups from the #805 review"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-07"
---

# scootlaunch panel visual follow-ups from the #805 review

Filed 2026-10-29. Serves **daily-drive** and the "beautiful and light" bar:
small polish and two small costs left after #805 (even panel spacing,
full-width rounded selection, icons inside it). Checked against `main` at
d9ba8ef54.

## The gap

Checked against `main` at ba66456c6 (2026-11-04). Items 1, 4 and 5 landed;
2 and 3 landed 2026-11-07 (below).

1. **Bottom padding exceeded top at 1.5x.** Mostly fixed already by #849
   (the fit remainder joins the top and bottom padding). A residue was left:
   when the rounded rows stood short of the scaled height by less than a row,
   the spare pooled under the last row (11 px text, 1.8x, four rows: 12 px
   above, 16 below). `fit_rows` now splits that spare too; bottom and top
   differ by at most the one pixel an odd remainder cannot split
   (`panel/padding_tests.rs`, every scale from 1x to 3x, six font sizes).
   RESOLVED.
2. **Square search field above the rounded selection pill.** Done
   2026-11-07: the field has no fill; what was square is its focus ring,
   drawn by `scootui::entry::paint::paint_entry`. The decision was to round
   it to the pill's language. `Geometry` gains a `radius` (0 is square, so
   the dmenu panel's bytes do not move); `Layout::field_geometry` and
   `list_geometry` pass `layout.radius`, the radius the highlight is filled
   with, so scale and look changes follow it. The ring is the new
   `Canvas::stroke_rounded` (outer coverage minus the concentric inner's,
   straight sides as runs, no allocation).
3. **The inverted match chip stays square** inside the rounded highlight on a
   selected row. Done 2026-11-07, same decision: the chip is filled with
   `fill_rounded_gradient` and cut at the row's text clip, as is the entry's
   select-all fill (the same square box, which would have poked out of the
   new rounded ring). A square chip corner did poke outside the pill (a span
   running to the text end, whose `lead` is less than the radius). The
   corner radius is `Geometry::radius` cut back so the arc ends above the
   line box (`text_box_radius`): at the pill's full radius the arc shaved the
   first letter's foot (the test font's `w`, DejaVu's `A` at 3x), so the chip
   is a tag with a gentler corner than the pill, not a pill within it.
   Pinned by `panel::round_tests` for every scale from 1x to 3x and five
   radii (no pixel the pill does not cover changes when the chip is added;
   the same test fails with the square chip, at (369, 40) at 1x) and by
   `entry::round_tests` (no glyph pixel of the chip lands in its cut corner;
   select-all stays inside the ring).

   Measured on linux-builder (release, uncommitted scratch tests). The
   primitives, 20,000 draws each: the rounded ring costs 1.2 us at 1x, 4.2 us
   at 2x, 9.7 us at 3x against the square ring's 0.08, 0.17, 0.25 us, and
   the rounded chip 0.7, 1.7, 2.9 us against 0.06, 0.19, 0.48 us. Whole
   `paint_panel` (ten rows, 40 entries, "appl" typed, 6 rounds of 400 draws),
   mean us per draw before / after: 1x 79.0 / 79.5, 2x 287.3 / 290.3, 3x
   640.7 / 646.1 (+0.6% to +1%, a few microseconds).
   The square look and the dmenu draw hash byte-identical before and after;
   only looks with a radius moved (the pins in `panel/bytes_tests.rs`).
   Screenshots: [before](launch-panel-visual-followups-before-2x.png),
   [after](launch-panel-visual-followups-after-2x.png).
4. **The icon cache built a key every draw.** The pixel clone was already
   gone (`Decoded.pixels` is an `Arc<[u8]>`); the owned `(String, u32)` key
   was the remaining allocation per visible row. The map is now keyed by side,
   then name, and a hit looks up by `&str`. A warm hit allocates nothing
   (`a_cache_hit_allocates_nothing`). Measured with `bench_cache_hits` (64
   warm hits, release): 5.6, 6.2, 5.8 us before; 5.7, 5.1, 5.4 us after, so the
   time barely moves and the per-redraw allocations are what is saved.
   RESOLVED.
5. **A click on the error line hit a hidden row.** `hit_row` now returns
   `None` while the spawn error is drawn. RESOLVED
   (`a_click_on_the_error_line_hits_no_row`).

## Not in this ticket

Config keys, the dmenu layout, and anything in the compositor.
