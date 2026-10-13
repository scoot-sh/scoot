---
title: "Extract `scootui`'s Wayland half: outputs, layer surfaces, shm, the poll loop (move only)"
status: "open"
area: "scootbar"
priority: "low"
blocked: "scootui-extract-draw"
milestone: "M7"
---

# Extract `scootui`: the Wayland half (move only)

Filed 2026-10-12 from the [shell-apps design](../../../dev/design/shell-apps.md#scootui-library-cratesscootui).
Serves **both**.

## What to do

A mechanical move behind a `wayland` feature, with **nothing new**:

- `outputs.rs`, `density.rs`, `region.rs`, `daemon/surfaces.rs`, `daemon/canvas.rs`;
- the Wayland half of the `poll` loop;
- `tests/common/` (the headless scoot and sway harness and the screenshot readers), as a
  test-support module.

New lifecycle code is `scootui-surfaces-on-demand`, a separate PR, so this one stays a
no-drift move.

## Acceptance

- scootbar's integration suites pass on headless scoot and sway.
- Ratchet rows within noise.
- `git diff -M` shows renames with only visibility and path edits.
- scootbg not moved (its release gate).

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

On-demand surfaces, the `xdg_toplevel` kind, the memfd name parameter.
