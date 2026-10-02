---
title: "Appearance follow-ups: hover token, per-module state colors, dot indicators, inactive-workspace colors"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M4"
resolved: "2026-10-01"
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

## What landed

Reference: [`docs/scootbar/cli.md`](../../cli.md) (`[colors]`, `[workspaces]`,
hover, the pill). All four items, in ticket order, each off by default:

- **`hover` token** (`Theme.hover`, `[colors] hover`, Stylix `base0A`): the
  render path tints a hovered bound module with it instead of `accent`, and
  the workspaces pill — which draws itself and had no hover — tints through
  a new `CustomDraw.hovered` flag (`tints_on_hover` now true; hoverable only
  with a binding, as before, so an unbound workspaces module repaints
  nothing new). Default is the accent value: every existing pixel is
  unchanged (all prior snapshots pass unmodified).
- **Per-module state colors** (`[workspaces] active-color`, `inactive-color`,
  both unset by default): the active pill's fill and inactive numbers' ink
  (and inactive dots'). Urgent and occupied colors are **not** built: the
  protocol has an `urgent` bit, but the bar ignores it
  (`daemon/workspaces.rs` reads only `Active`) and scoot never sends it
  (no `xdg_activation` yet) — so there is nothing to drive the colors
  with. The work is read-the-bit plus send-the-bit; documented in cli.md.
- **Dots** (`[workspaces] display = "numbers" | "dots"`, default numbers):
  one ordinary `o` cell per workspace in the view text, so measuring and
  the hit test walk the dots' own places unchanged; `custom_draw` fills one
  disc each (active like the pill, the rest dim) and draws no text. `query`
  still reports the numbers. `disc` with dots is refused.
- **Disc** (`[workspaces] disc`, off by default, refused unless
  `pill-shape = "circle"` showing numbers): a new defaulted module hook,
  `Module::span_extra` (`Measure`: output, view, measurer, em, scale, bar
  height), consulted once at measure time by `Scene::update`. The workspaces
  circle grows its own span by disc-diameter less the active number's width,
  so the pill the circle grows into fits — no text hack, no hit-test
  change (positions are span-relative and the text is untouched).

## Evidence

Captured on the dev VM (`ssh -p 2222 dev@localhost`, tree at `/mnt/scoot`,
the 9p mount of this checkout), at the commit carrying this entry:

- `cargo nextest run -p scootbar` — 750 run: 749 passed, 1 failed. The
  failure is `agent::layout_rectangles_are_where_a_click_lands_on_two_outputs_at_two_scales`
  (headless-2 at scale 1.5 vs 1.0), proven pre-existing and environmental
  during the exec-keep work: it fails identically with this lane's changes
  stashed. Nothing here touches outputs, scales, layout or agent code.
- New tests, all passing: theme default (`hover_is_the_old_tint_until_set`),
  config parse/refusals (`hover`, `active-color`, `inactive-color`,
  `display`, `disc`), workspaces pixel units (hovered pill, configured
  colors, dots discs + dot hit test, disc span growth and its five no-growth
  cases), and 10 new snapshots at 1x and a fractional scale, each reviewed
  pixel-by-pixel before blessing (right-module ink in the hover token, pill
  in hover/active colors, dots with zero text ink, square disc bounding
  boxes): `bar-hover-*`, `workspaces-pill-hover-*`, `workspaces-colors-*`,
  `workspaces-dots-*`, `workspaces-disc-*`.
- `cargo clippy -p scootbar --all-targets -- -D warnings` — clean (also
  with `--no-default-features` and each module feature alone).
- `cargo fmt --check -p scootbar` — clean.
- `nix build .#scootbar` and `nix build .#checks.aarch64-linux.scootbar-modules`
  (the CI nix-module check, which runs `daemon --check` over the rendered
  files) — both pass on the dev VM.
- Ratchet (numbers in cli.md and [lightest.md](lightest.md)): dots cost one
  small maximally-rounded fill each, ~6 us at a 50-pixel test em, linear in
  the count (25/49/100 us for 4/8/16, release, dev VM); a grown disc ~8.5 us
  against 5 us for the plain pill at the same scale. No new fd, timer or
  wakeup from any of the four (the contract test holds the source budget),
  so idle is unchanged by construction; the default look's pixels are
  byte-identical. No full hardware bench: recolors and bounded fills need
  no new bench row beyond these micro-costs.
