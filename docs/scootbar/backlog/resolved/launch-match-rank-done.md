---
title: "scootlaunch: fuzzy matching and frecency, `--query --json`"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-18"
---

# scootlaunch: matching and ranking

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#matching-and-ranking). Serves **both**.

## What to do

- The hand-rolled fuzzy matcher, with field weights and the character-mask pre-filter.
- The frecency file in `$XDG_STATE_HOME/scootlaunch/history`: bounded, atomic, tolerant of
  damage.
- The combined ordering, with the frecency bonus capped.
- `--query TEXT --json`.

## Acceptance

- Ordering tests: a prefix beats a scattered match, and a habit never beats a much better match.
- Zero allocation per query after warm-up.
- At most 1 ms per keystroke for 3,000 entries on the M2.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

The UI.
