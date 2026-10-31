---
title: "scootnotify's click area is each card's rectangle: a pill's cut corners still take clicks"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-31"
---

# scootnotify's click area is each card's rectangle: a pill's cut corners still take clicks

Filed 2026-10-30, found building #807 slice 3 (card styles). Serves
**computer use**: a click-after-screenshot agent aiming at the desktop in a
card's cut corner hits the card instead.

## The gap

`Popup::set_input_region` (`crates/scootnotify/src/popup.rs`) adds one
rectangle per card (`Card::rect`, or `Motion::input_rect` mid-motion).
A rounded card (8 px by default) has always taken clicks in its cut
corners; `card.shape = "pill"` cuts 36 px corners, so about 280 px² per
corner of visible desktop takes the click.

## What to do

Build the region from `scootui::region::input_rects(width, height,
radius, out)`, which scootbar already uses for its own shape, per card at
layout (a pooled `Vec`, not per repaint), offset to the card's origin.
Mid-motion the rectangles follow the pose (scaled and translated), so the
rows need the same transform `Motion::input_rect` applies to the rectangle.
Pin it with a headless-scoot click in a pill's corner reaching the window
behind.

## Not in this ticket

The hit test for pointer hover (`Popup::hit`), which should follow the same
shape once the region does.

## Resolution

Landed 2026-10-31 (PR for #807, follow-up). The input region is each card's
outline, `Look::card_radius(file)` (the one source the paint also uses), as
the rows of `scootui::region::push_input_rects`: at rest offset to the
card's origin, in motion through the pose (`Motion::push_input_rects`: the
card's scaled size, translation and shift, the radius scaled with it, clipped
to the surface). Into one pooled list; the `wl_region` is created and sent
only when the list differs from the surface's (`popup/region.rs`), which
also stops the per-draw region churn at rest.

Pinned by `a_click_in_a_cut_corner_reaches_the_window_behind` in
`crates/scootnotify/tests/popups.rs` (headless scoot, a pill's corner click
focuses the window behind; a `radius = 0` card keeps the click and is
dismissed), and by unit tests on the rows.

`Popup::hit` is unchanged: a real pointer never enters a cut corner now (the
compositor delivers it to the window behind), so only the pose-unprojected
hover mid-motion could still count a corner as the card's.
