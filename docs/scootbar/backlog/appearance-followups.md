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

- **A `hover` token, and per-module token-per-state overrides.** Hover exists now
  ([pointer-and-interactions](resolved/pointer-and-interactions-done.md), M4): a module
  with a binding is drawn in the existing `accent` token while the pointer is over it,
  and only that module's span is redrawn. There is no `hover` token of its own, so the
  tint cannot differ from the accent (and the workspaces pill, which draws itself, has no
  hover at all). A dedicated token, and per-module overrides now that a module can enter
  one, is what is left. Note the hover tint is on by default for bound modules, which the
  rule below ("behind an option that is off by default") would not allow for a new look:
  the maintainer ruled nothing on it yet, and the PR that landed it says so.
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

Take them in the order above (pointer-and-interactions, which this waited on, is done); each stays
behind an option that is off by default, and each lands with snapshots at 1x and
a fractional scale and a row in the [resource ratchet](lightest.md) if it draws.

## Not in this ticket

The opt-in animation question, dropped in appearance (no frame callbacks were
added); it returns only with a measurement that says it is free. Per-output
overrides are [multi-output](resolved/multi-output-done.md).
