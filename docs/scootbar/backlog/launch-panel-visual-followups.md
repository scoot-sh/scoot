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

1. **Bottom padding exceeds top by a few device pixels at 1.5x.** The panel's
   logical height comes from `Layout::new(em, radius, 1.0).height(rows)`
   (`crates/scootlaunch/src/panel.rs` near line 284), is rounded once, and
   `paint_panel` scales it to device pixels (`scale_len`) while the rows are
   laid out by `device_layout(look, scale)` from the device em, so the
   content ends short of the buffer. #805 measured about 9 px of bottom
   padding against 15 px on top for `music-desk` at 1.5x, and called it
   pre-existing in kind. Compute the panel height from
   `device_layout().height()` instead.
2. **Square search field above the rounded selection pill.** The search field
   keeps square corners while the selected row is a rounded pill.
3. **The inverted match chip stays square** inside the rounded highlight on a
   selected row (visible in the `moonrise` screenshot #805 added to the docs).
   Pre-existing chip behavior.
4. **The icon cache copies pixels and builds a key every draw.**
   `crates/scootlaunch/src/icons.rs`: `Cache::get` builds
   `(name.to_string(), side)` on every call, and a hit returns
   `Icon::Image(decoded.clone())`, cloning the pixel `Vec`. The visible rows
   pay this on each redraw. Hold the pixels in an `Rc`/`Arc` and look up
   without allocating a key. Pre-existing; #805's bench adds no allocation of
   its own.
5. **A click on the error line over a populated list hits a hidden row.**
   When `error` is set, `paint_panel` draws only the error line where the
   first row would be and returns before drawing any row, but
   `hit_row` still maps the click onto `ui.list().visible_range()`, so a click
   on that line selects or launches a row that is not drawn. Pre-existing.
   (The error shows for 2 s after a failed spawn.)

## What to do

Fix 1 first (it is visible on every 1.5x output), with a test that the
bottom padding equals the top padding at 1x, 1.5x and 2x. Fix 5 by having
`hit_row` return `None` while the note is drawn. The rest are polish and a
small hot-path saving; measure 4 against the `paint_panel` bench before and
after.

## Not in this ticket

Config keys, the dmenu layout, and anything in the compositor.
