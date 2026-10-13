---
title: "Extract `scootui`'s Wayland half: outputs, layer surfaces, shm, on-demand surfaces"
status: "open"
area: "scootbar"
priority: "low"
blocked: "scootui-extract-draw"
milestone: "M7"
---

# Extract `scootui`: the Wayland half

Filed 2026-10-12 from the [shell-apps design](../../../dev/design/shell-apps.md#scootui-library-cratesscootui).
Serves **both**.

## What to do

Move `outputs.rs`, `density.rs`, `region.rs`, `daemon/surfaces.rs`, `daemon/canvas.rs` and
the Wayland half of the `poll` loop behind a `wayland` feature; add the memfd name
parameter to `scootbg-mem` (today every buffer is `scootbg-wallpaper`,
`crates/scootbg-mem/src/shm.rs:173`). New in this PR, as its first real user is the next
ticket: an **on-demand** layer-surface lifecycle (created with content, destroyed with
buffers unmapped when empty) and a full-screen `xdg_toplevel` surface kind (scootgreet).
Move `tests/common/` (headless scoot and sway harness, screenshot readers) into a
test-support module.

## Acceptance

scootbar's integration suites pass on headless scoot and sway; ratchet rows within noise;
the on-demand lifecycle has a test that creates and destroys a surface 1,000 times with
fds and RSS flat; scootbg not moved (its release gate, [design](../../../dev/design/shell-apps.md#scootui-library-cratesscootui)).

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- `/proc/PID/maps` showing the buffers named per app.

## Not in this ticket

Moving scootbg onto `scootui` (a later ticket after two apps prove it).
