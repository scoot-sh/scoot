---
title: "scootlaunch: icon-theme lookup with inheritance, PNG first, SVG through `resvg`"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-19"
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

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/scootlaunch/panel.md` with the icon column, icon theme lookup and the size/memory budget; `look` icon role if added. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

Window icons.

## Resolution

Shipped in PR #679 (`feat/launch-icons` branch): theme lookup with
PNG first and `resvg` SVG, the icon column, and the `[icons] theme`
look role.

What landed:

- `scootlaunch/src/icons.rs`: Icon Theme lookup (configured theme,
  `Inherits` chain, `hicolor`, then bases), PNG preferred, SVG via
  `resvg` 0.48 no-default-features (default-on `svg`), caps (512 KiB,
  20000 nodes, 256 px render, no external refs), visible-rows-only
  loads, 64-entry 1 MiB cache, letter tile fallback, device-pixel
  sides. Hostile inputs fall back to tiles, never panic or stall.
- Panel (`panel.rs` + `ui.rs`): icon column shifting entry and list
  text together (`Geometry::text_x`), entry shows the selection icon,
  fractional scale draws at device pixels. Keystroke path untouched.
- Look: `scoot-look` renders `[icons] theme` (default `Adwaita`),
  allowlisted as `icons.theme`; `scootlaunch` reads it.
- Docs: `site/src/content/docs/scootlaunch/panel.md` icons section,
  budgets and tile symptom; screenshots refreshed (post-inset, tiles).

Evidence (builder, x86_64, exact heads in the PR; aarch64 via CI):
`cargo nextest run -p scootlaunch` 291 passed; `cargo test`
opposite green; `scoot-look` 182 passed; `clippy -- -D warnings`
and `fmt --check` clean; fuzz workspace check clean; `cargo tree
-i libc` only via `xkbcommon`; release 1446648 B without `svg`
(budget 1.8 MB) and 2167544 B with it (waiver row); `ldd` only
std plus `libxkbcommon`. Mutation-proved: `format!` in rerank reddens
the alloc pin, icons alloc leaves it green; PNG-first swap reddens
its test.
