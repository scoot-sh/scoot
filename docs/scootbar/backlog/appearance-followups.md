---
title: "Appearance follow-ups: hover token, per-module state colors, dot indicators, inactive-workspace colors"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M4"
---

# Appearance follow-ups: hover token, per-module state colors, dot indicators, inactive-workspace colors

Filed 2026-09-30, from the remainder of [appearance](resolved/appearance-done.md),
which is resolved. Serves **daily-drive**: none of this is needed to use the bar,
and the engineering priority order (speed and correctness before polish) applies
to each item, so each is measured or cut.

## The gap

What [appearance](resolved/appearance-done.md) left, none of it started:

- **The `hover` token and per-module token-per-state overrides.** The bar tracks
  the pointer's position for a press (`pointer_on`) but draws no hover: a hover
  state needs pointer motion to redraw the module under it and back, which is
  [pointer-and-interactions](pointer-and-interactions.md) (M4). A token no module
  can enter has no honest test beyond forcing it. The four classes (`normal`,
  `warn`, `urgent`, `muted`) already map to tokens, and the only module that sets
  one (workspaces) does not, so per-module overrides have nothing to override yet.
  Do this with or after pointer-and-interactions.
- **Dot-style workspace indicators**: a row of small dots in place of the numbers.
  A different look from the pill and circle shapes, so a `workspaces` display mode,
  not a `pill-shape`. Needs a hit test that follows the dots and a fixed small
  cost per workspace.
- **Colors for inactive workspaces, or per-state pill colors** (an urgent
  workspace, an occupied one).
- **A disc at any padding.** A circle's diameter is limited to the module's span
  (text plus `padding` each side), so a single digit on a small `padding` is an
  oval. Growing the module's own span to the diameter is the alternative, if a
  disc at any padding is wanted (documented in
  [cli.md](../cli.md#the-active-workspaces-pill)).
- **The default margin matching scoot's gap**: a product call, not an
  implementation. The default look is flush, square and opaque; the example
  `[bar]` snippet with `margin` equal to scoot's `[layout] gap` is in
  [cli.md](../cli.md#margins).

## What to do

Take them in the order above only as pointer-and-interactions allows; each stays
behind an option that is off by default, and each lands with snapshots at 1x and
a fractional scale and a row in the [resource ratchet](lightest.md) if it draws.

## Not in this ticket

The opt-in animation question, dropped in appearance (no frame callbacks were
added); it returns only with a measurement that says it is free. Per-output
overrides are [multi-output](resolved/multi-output-done.md).
