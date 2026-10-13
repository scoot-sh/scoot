---
title: "Extract `scootui`: move scootbar's pure drawing, widgets and icons into a library crate"
status: "open"
area: "scootbar"
priority: "low"
blocked: "maintainer go-ahead on the shell-apps design (dev/design/shell-apps.md, Decisions needed 1)"
milestone: "M7"
---

# Extract `scootui`: the drawing half

Filed 2026-10-12 from the [shell-apps design](../../../dev/design/shell-apps.md#scootui-library-cratesscootui).
Serves **both**: every shell app draws through it, and scootnotify is the second consumer
[extract-scootui](extract-scootui.md) waited for.

## What to do

A mechanical move, no logic edits: `paint.rs`, `color.rs`, `theme.rs`, `text.rs`,
`font.rs`, `modules/ellipsis.rs`, `print.rs`, the popup's pure half (`popup/`), and
`icon/` into `crates/scootui` (features: always, `widgets`, `icons`, `png`), with
`testfont.rs` and `snapshots.rs` as test support. scootbar depends on it. Fuzz targets
that compile these files by `#[path]` move with them, corpora intact. Add the CI classify
output and arms for `crates/scootui/` (fanning out to scootbar) in this PR, as CLAUDE.md
requires, and `scootui` stays `#![forbid(unsafe_code)]`.

## Acceptance

- scootbar's snapshot tests pass with **no snapshot re-blessed**; every
  `count_allocations == 0` test passes unchanged.
- scootbar's ratchet rows (stripped size, `.text`, idle RSS/PSS, wakeups, fds), M6 method,
  against `main`: within noise, or a written waiver ruling.
- scootbg untouched.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- `git diff -M --stat` showing the moves as renames.

## Not in this ticket

The Wayland scaffolding (`scootui-extract-wayland`), the D-Bus client (`scoot-dbus-extract`),
any new widget.
