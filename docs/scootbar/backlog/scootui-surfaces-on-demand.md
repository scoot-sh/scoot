---
title: "`scootui` surfaces: on-demand layer surfaces, a full-screen `xdg_toplevel` kind, memfd names"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M7"
---

# `scootui`: on-demand surfaces

Filed 2026-10-12 in the #579 fix round, split out of the Wayland move so the move stays
mechanical. Serves **both**.

## What to do

- An **on-demand** layer-surface lifecycle: created when there is content, destroyed with its
  buffers unmapped when empty, so no surface is held at idle.
- A full-screen `xdg_toplevel` surface kind (scootgreet: cage 0.3.1 has no layer-shell).
- The memfd name parameter in `scootbg-mem`. Today every buffer is `scootbg-wallpaper`
  (`crates/scootbg-mem/src/shm.rs:173`).

## Acceptance

- A test creates and destroys an on-demand surface 1,000 times with fds and RSS flat.
- The `xdg_toplevel` kind maps fullscreen under headless scoot and sway.
- `/proc/PID/maps` shows buffers named per app.
- scootbar unchanged (its ratchet rows within noise).

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

Keyboard input.
