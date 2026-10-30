---
title: "scootnotify's click area is each card's rectangle: a pill's cut corners still take clicks"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
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
