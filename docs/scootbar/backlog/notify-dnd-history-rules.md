---
title: "scootnotify: do not disturb, the history ring and `restore`, `[[rule]]` with `exec`"
status: "open"
area: "scootbar"
priority: "low"
blocked: "notify-control"
milestone: "M7"
---

# scootnotify: DND, history, rules

Filed 2026-10-12 in the #579 fix round, split out of `notify-control`. Serves **daily-drive**.

## What to do

From the [design](../../../dev/design/scootnotify.md#do-not-disturb-history-rules):

- the DND mode and `msg dnd`;
- the in-memory history ring, with `msg history` and `msg restore`;
- `[[rule]]` matching and actions, with `exec` under the bar's exec bounds (at most 4 running,
  argv, process group, pidfd).

## Acceptance

- DND holds and releases correctly, with critical cards bypassing it.
- History caps and `transient` exclusion.
- Rule order and merging.
- A rule `exec` flood bounded at 4 children with RSS and fds flat.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

The bar feed.
