---
title: "scootlaunch: a windows mode over wlr-foreign-toplevel-management"
status: "open"
area: "scootbar"
priority: "low"
blocked: "launch-nix"
milestone: "M7"
---

# scootlaunch: windows mode

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#windows-mode). Serves **daily-drive**. Low,
and deliberately after the profile switch: it is not needed to replace fuzzel.

## What to do

`--windows` lists **and** activates through `zwlr_foreign_toplevel_handle_v1`, which carries
title, app_id and state, with `activate(seat)`. `ext-foreign-toplevel-list-v1` is not used: its
handles share nothing with wlr's, so it could not be activated from. Icons come from `app_id`
matched to a desktop id or `StartupWMClass`.

## Acceptance

Headless tests on scoot and on sway: three windows listed, and the chosen one focused.

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/scootlaunch/panel.md` with the windows mode, its key and what it lists. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

Window actions beyond activate.
