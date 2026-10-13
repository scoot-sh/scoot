---
title: "scootlaunch: icon-theme lookup with inheritance, PNG first, SVG through `resvg`"
status: "open"
area: "scootbar"
priority: "low"
blocked: "launch-ui"
milestone: "M7"
---

# scootlaunch: icons

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#architecture); split in the #579 fix round.
Serves **daily-drive**.

## What to do

- Icon Theme spec lookup: the configured theme, its `Inherits` chain, `hicolor`, then the base
  directories.
- A PNG of the right size is preferred when one exists; SVG is rasterized through `resvg` (a
  default-on `svg` feature) only when none does.
- SVG caps: a node-count cap, a render-size cap, and no external references.
- Only the visible rows' icons are loaded.

## Acceptance

- The size row of `svg` is measured, for the maintainer's waiver ruling.
- A letter tile appears without the feature.
- Icons render at device pixels on a fractional output.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

Window icons.
