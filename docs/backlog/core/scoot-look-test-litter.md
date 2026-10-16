---
title: "scoot-look test litter in the system temp dir"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# scoot-look test litter in the system temp dir

Filed 2026-10-16. Serves **daily-drive** (a slow temp-dir leak on dev
machines that run the suite often).

## The gap

`tmp_case` in `crates/scoot-look/src/main.rs` (the `#[cfg(test)]`
module) creates
`$TMPDIR/scoot-look-main-{name}-{pid}` per case and never removes it.
Every `cargo test -p scoot-look` run leaves the directories behind
(the FIFO/symlink/catalog cases). Pre-existing, not from any looks
ticket: spotted in the `look-nix-catalog` review (PR #634).

## What to do

Remove each case dir when the test finishes (a guard/drop helper, or
`tempfile::TempDir` like the `scootctl` look fixtures use), and keep
the FIFO non-hang regression covered. Tests must never touch the real
HOME/XDG (scratch HOME plus every `XDG_*` plus `TMPDIR`).

## Not in this ticket

Any behavior change to `scoot-look` itself.
