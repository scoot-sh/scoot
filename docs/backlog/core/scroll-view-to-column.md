---
title: "A core action that scrolls the view to a column, so a swipe settles on it"
status: "open"
area: "core"
priority: "medium"
blocked: null
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
