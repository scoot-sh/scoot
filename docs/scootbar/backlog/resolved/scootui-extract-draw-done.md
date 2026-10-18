---
title: "Extract `scootui` and `scoot-rt`: move scootbar's pure drawing, widgets, icons and print/runtime-dir helpers"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-18"
---

# Extract `scootui` and `scoot-rt`: the drawing half

Filed 2026-10-12 from the [shell-apps design](../../../dev/design/shell-apps.md#scootui-library-cratesscootui).
Serves **both**. Held until one of the three gates says go, so that no refactor is spent on a no.

## What to do

A mechanical move with no logic edits:

- **Into `crates/scootui`**: `paint.rs`, `color.rs`, `theme.rs`, `text.rs`, `font.rs`,
  `modules/ellipsis.rs`, the popup's pure half (`popup/`) and `icon/`. Features: always,
  `widgets`, `icons`, `png`. `testfont.rs` and `snapshots.rs` move as test support.
- **Into `crates/scoot-rt`**: `print.rs` and `runtime_dir` (`control/paths.rs:89-96`). This is
  the leaf both `scootui` and `scoot-dbus` depend on
  ([why](../../../dev/design/shell-apps.md#scoot-rt-library-cratesscoot-rt-the-leaf-both-depend-on)).
  scootbar's `dbus/` and `control/` call it from their current place for now.
- Fuzz targets that compile these files by `#[path]` move with them, corpora intact.

In this PR:

- CI classify outputs and arms for `crates/scootui/` and `crates/scoot-rt/` (fanning out to
  scootbar), as CLAUDE.md requires;
- CLAUDE.md scope entries for `scoot-rt` (`scootui` is already listed);
- `#![forbid(unsafe_code)]` in both crates.

## Acceptance

- scootbar's snapshot tests pass with **no snapshot re-blessed**; every
  `count_allocations == 0` test passes unchanged.
- scootbar's ratchet rows (stripped size, `.text`, idle RSS/PSS, wakeups, fds), by the M6 method
  against `main`: within noise, or a written waiver ruling.
- scootbg untouched.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- `git diff -M --stat` showing the moves as renames.

## Not in this ticket

The Wayland scaffolding (`scootui-extract-wayland`), the control socket
(`scootui-extract-control`), the D-Bus client (`scoot-dbus-extract`), any new widget.

## Resolution

Landed in PR #662 (commit `aa57c45e6` on branch
`refactor/scootui-extract-draw`, against `origin/main` at `7bc84961`).

What landed, beyond the ticket's list:

- `modules/class.rs` (`Class`, `MAX_TEXT`) moved to `scootui::class` with a
  re-export from `scootbar::modules`, since `theme` and `ellipsis` need it.
- `OutputId` moved to `scootui::output` with a re-export from
  `scootbar::outputs`, since `popup::hover` needs it.
- `Text::new`, `Text::icons_cached`, `Cache::cached` and
  `Corners::coverage_at` are now available to dependents (not `cfg(test)`
  only), since scootbar's tests use them through the new crates.
- Popup/tip tests carry local scale stand-ins (same arithmetic) until
  `scootui-extract-wayland` moves `density`.
- `snapshots.rs` stays in scootbar: its scenes test bar rendering
  (`render`/`layout`/`modules`), which does not move in this ticket.
  `testfont.rs` moved as test support.
- `scootui` gains a `tray` feature (for `icon/tray.rs`) alongside the
  ticket's `widgets`, `icons` and `png`, mirroring scootbar's gates.

Evidence (dev VM, aarch64, `CARGO_TARGET_DIR=/dev/shm/w10-extract-target`):

- `cargo nextest run -p scoot-rt -p scootui -p scootbar`: 1519 passed.
- Snapshots: 17 passed, no re-bless. Allocation pins: 8 passed.
- Clippy matrix: default, `--no-default-features`, each alone, `popup`+each.
- Fuzz budgets: format 1M/18s, payload 2M/31s, volume 2M/6s,
  network 2M/97s, dbus 1M/36s, tzif 5M/12s, all clean.
- Ratchet (release, `main` at `7bc84961` vs branch): stripped size
  3019488 vs 3019488 (identical); `.text` 2443880 vs 2444520 (+640);
  idle RSS 4652 vs 4648, PSS 2791 vs 2787, wakeups 2 vs 2, fds 8 vs 8.
- `git diff -M --stat` shows the moves as renames.
- scootbg untouched (`git diff --name-only` has no `scootbg`).
