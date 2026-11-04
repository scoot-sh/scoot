---
title: "scootlaunch panel visual follow-ups from the #805 review"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
---

# scootlaunch panel visual follow-ups from the #805 review

Filed 2026-10-29. Serves **daily-drive** and the "beautiful and light" bar:
small polish and two small costs left after #805 (even panel spacing,
full-width rounded selection, icons inside it). Checked against `main` at
d9ba8ef54.

## The gap

Checked against `main` at ba66456c6 (2026-11-04). Items 1, 4 and 5 landed;
2 and 3 stay open.

1. **Bottom padding exceeded top at 1.5x.** Mostly fixed already by #849
   (the fit remainder joins the top and bottom padding). A residue was left:
   when the rounded rows stood short of the scaled height by less than a row,
   the spare pooled under the last row (11 px text, 1.8x, four rows: 12 px
   above, 16 below). `fit_rows` now splits that spare too; bottom and top
   differ by at most the one pixel an odd remainder cannot split
   (`panel/padding_tests.rs`, every scale from 1x to 3x, six font sizes).
   RESOLVED.
2. **Square search field above the rounded selection pill.** Open. The field
   has no fill; what is square is its focus ring, drawn by
   `scootui::entry::paint::paint_entry`, which the bar and the other apps
   share. Rounding it is a visual call and a shared-crate change, so it needs
   a decision on the look first, not a drive-by edit here.
3. **The inverted match chip stays square** inside the rounded highlight on a
   selected row. Open, for the same reason: the chip is filled by the shared
   list painter in `scootui`.
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

## What to do

Decide the look for items 2 and 3 (round the field's ring and the chip, or
keep them square on purpose), then change `scootui`'s painters with a
byte-pin update for each app that draws them.

## Not in this ticket

Config keys, the dmenu layout, and anything in the compositor.
