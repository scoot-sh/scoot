---
title: "scootlaunch: a windows mode over the standard foreign-toplevel protocols"
status: "open"
area: "scootbar"
priority: "low"
blocked: "launch-ui"
milestone: "M7"
---

# scootlaunch: windows mode

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#windows-mode). Serves **daily-drive**.

## What to do

`--windows`: list from `ext-foreign-toplevel-list-v1`, activate with
`zwlr_foreign_toplevel_handle_v1.activate`, icons by `app_id` to desktop id or
`StartupWMClass`. Standard protocols, not scoot IPC (CLAUDE.md).

## Acceptance

Headless tests on scoot and on sway: three windows listed, the chosen one focused.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

Window actions beyond activate.
