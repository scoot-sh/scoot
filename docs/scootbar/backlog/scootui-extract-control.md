---
title: "Extract `scootui`'s control socket: framing, paths, claim and connection bounds (move only)"
status: "open"
area: "scootbar"
priority: "low"
blocked: "scootui-extract-draw"
milestone: "M7"
---

# Extract `scootui`: the control socket (move only)

Filed 2026-10-12 in the #579 fix round: no ticket moved `control/`, yet `notify-control`
assumes it exists. Serves **both**: scootnotify's `msg` and `subscribe`, and agents.

## What to do

- Move `crates/scootbar/src/control/{framing,paths,claim,conn}.rs` and their bounds
  (`MAX_CONNECTIONS`, `MAX_SUBSCRIBERS`, the write-stall deadline; `control/mod.rs:57-65`) into
  a `control` feature of `scootui`.
- `runtime_dir` already lives in `scoot-rt`.
- The socket name becomes a parameter instead of `scootbar-…`. That is the only API change,
  and scootbar passes its current name.
- scootbar's request handling (`protocol.rs`, `respond.rs`) stays in scootbar.

## Acceptance

scootbar's control tests (`control/tests.rs`, `subscribe_tests.rs`) and its socket bounds tests
pass unchanged, and the ratchet rows are within noise.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

scootnotify's verbs.
