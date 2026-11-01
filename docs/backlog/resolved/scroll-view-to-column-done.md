---
title: "A core action that scrolls the view to a column, so a swipe settles on it"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-01"
---

# A core action that scrolls the view to a column

Filed 2026-10-31 (touchpad swipe gestures, #820 / PR #832). Serves
**daily-drive**: the gesture feels half-finished without it.

## The gap

Issue #820's headline is that a three-finger swipe "settles on the nearest
column". The swipe follows the fingers and focuses the nearest column, but the
only way to move the view is `focus-column`, and the core scrolls the view
only as far as needed to show the focused column
(`layout::scroll_into_view`). With the default layout, two columns are fully
visible, so focusing the neighbour changes nothing about the view: the strip
follows the fingers, then eases back to where it was, and only the focus ring
moves.

Evidence (real `--tty`, synthetic touchpad, PR #832): focus 1 -> 2 after a
swipe of about 0.75 column, with `scoot msg windows` reporting identical
window x values before and after (`x: 12` and `x: 806`, both visible). The
mid-swipe screenshot shows the strip pulled about 540 px; the settled one shows
it back at rest.

So "settles on the nearest column" is delivered for focus and not for the view.

## What to do

A core action (and event handling) that sets the view so a given column's left
edge sits at the usable edge, clamped to the strip like every other scroll:
`scroll-view-to-column`, or a `focus-column` variant that aligns. Then the
gesture fires it on release instead of stepping `focus-column`, and the settle
re-bases onto the aligned targets as it already does. `scoot-core` is
fuzz-tested; this needs its own tests there (clamping at both ends, one
column, fullscreen and maximized columns, a view already aligned).

## Not in this ticket

Centering columns, or any change to what plain `focus-column` does.

## Resolved 2026-11-01

Landed `Action::ScrollViewToColumn(usize)` (`scroll-view-to-column N`, wire `scroll_view_to_column`): focuses column N (0-based, strip order) and lines the view up with its left edge, clamped to `[0, strip-view]`. Out of range / no output / no columns does nothing. The gesture release fires it once instead of stepping `focus-column`; the settle re-bases onto the aligned targets as before. No protocol bump (additive tag, following `SetColumnWidth`); no default bind. Tests: 10 new `scoot-core` column tests, fuzz alphabet extended in `invariants.rs`, IPC round-trip + `scoot msg action` + bind parse/emit pins, compositor gesture test asserting the settled column sits at the usable edge. Hardware: real `--tty` synthetic pad, 3 foot windows, ~0.75-column swipe 1->2 on both `--renderer cpu` and `gpu`: `windows-before` (1 at x12 focused, 2 at x806) -> `windows-after1` (1 at x-782, 2 at x12 focused) — the view moved a full column instead of easing back. Screenshots rest/mid/after1/settled for both renderers.
