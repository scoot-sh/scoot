---
title: "The five shipped looks as look.toml under looks/, validated in CI"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-15"
---

# The five shipped looks as look.toml under looks/, validated in CI

Filed 2026-10-12, child of [`live-looks`](live-looks.md). Serves **both**:
the five looks are what users pick and what the goldens screenshot.

## The gap

The five looks exist twice and nothing compares them: the Nix registry
(`desktop.nix:1931-2147`, 3+6 colors) and the example directories, which mix
the look with a layout and commands (`docs/examples/moonrise/bar.toml:19-21,58-86`).
`theming.md:208-209` claims "a check renders every look into every target
and fails on a missing role"; no such check exists. And a change under
`docs/` runs no CI beyond the ReGreet gate (`.github/workflows/ci.yml:236-246`).

## What to do

- Lift each look into `looks/<name>/look.toml` (D1), with no invented color:
  the registry's values, foot's 16 hues into `[terminal]`, and (D15 is
  decided yes, 2026-10-14) the examples' style values: ring widths,
  padding, spacing, separator. **Not yet radius or bar opacity** (D15's
  ordering, R3): this ticket runs before `look-layer-scoot`'s pixman
  benchmark, so it lifts `[shape] radius = 0` and `[opacity] bar = 1.0` with
  the examples' values beside them as comments; `look-layer-scoot` raises
  them only if its benchmark passes. Gap stays in the example layout
  configs (D7). The examples keep their layout and lose their colors (they
  point at the look).
- A CI path filter for `looks/` in the classify job (the way scootbg's is),
  and a test in `scoot-look` that validates and renders every directory
  there: this is the gate that makes `theming.md:208-209` true.
- A Nix check that each `look.toml` agrees with its registry entry until
  `look-nix-retire-registry`, over the fields both have (the 3 appearance
  and 6 bar colors, `isDark`/`polarity`, `fonts`, `nightTemp`, the
  wallpaper); `[shape]`, `[opacity]` and `[terminal]` have no registry
  counterpart and are compared with the example files instead.
- Put the `dim` and `urgent` contrast numbers (music-desk dim 1.57 and
  urgent 2.88, vinyl-sunset dim 2.18) in front of the maintainer: retune or
  accept.
- Docs: the theming page's look table points at `looks/`.

## Evidence to record

The five files, the CI job running on a look-only change (and a deliberately
broken look failing it), and the registry-equality check.

## Resolution (PR #610, branch `feat/look-shipped`)

`looks/<name>/look.toml` for all five looks, from the registry's values,
foot's 16 hues, and the examples' ring widths, padding, spacing and
separator, with `[shape] radius = 0` and `[opacity] bar = 1.0` held and the
examples' values beside them as comments (R3). Each look carries its
wallpaper as `wallpaper.png` beside the manifest (byte-identical copies;
vinyl-sunset keeps the flat background color since its illustration cannot
be committed). The `scoot-look` test over `looks/`
(`crates/scoot-look/tests/shipped.rs`) validates and renders every look
there and locks the R3 hold plus the known dim/urgent warnings (music-desk
dim 1.6 and urgent 2.9, vinyl-sunset dim 2.2; design table 1.57, 2.88,
2.18, ratios round to one decimal: reported, never refused;
retune or accept is the maintainer's call). The classify job gains a
`looks/` path filter running only the look job. `nix/looks-shipped-check.nix`
pins each manifest against the registry through the theme helper's own
import (plus cursor/icon theme, wallpaper bytes by hash, and the style
tokens against the example files); `nix/tests.nix` forces it at eval on
every platform. The theming page's look table points at `looks/`, naming
both the Nix and the non-Nix path. The examples keep their own copies of
the values until the look layer lands (stripping them now would break
`install.sh --look` with no working layer to point at), and music-desk's
`popup-radius` plus the foot terminal sizes stay in the examples with them.

## Not in this ticket

Third-party leaves (`apps/` files move in the leaves tickets).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
