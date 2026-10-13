---
title: "scootlaunch: the desktop entry index, Exec expansion and `--list --json`"
status: "open"
area: "scootbar"
priority: "low"
blocked: "GO verdict recorded in launch-prototype (a human removes this clause)"
milestone: "M7"
---

# scootlaunch: the desktop entry index

Filed 2026-10-12 from the [scootlaunch design](../../../dev/design/scootlaunch.md#the-desktop-entry-index). Serves
**computer use** (`--list --json` works with no compositor) and **daily-drive**.

## What to do

Create `crates/scootlaunch`, with its CI classify output and job. The modules are pure:

- data dirs and ids, with precedence and `Hidden` masking;
- the format and its escapes;
- localized keys, in the spec's order;
- `OnlyShowIn`/`NotShowIn` and `TryExec`;
- actions;
- the Exec quoting and field codes, including the must-reject cases;
- the cache, keyed by canonical path, inode and mtime (the NixOS store-mtime case);
- symlinks followed, with a visited set.

Plus `scootlaunch --list [--all] --json`.

## Acceptance

- Unit tests for every rule in the design.
- Fuzz targets for the parser and the Exec expander, seeded from real NixOS, Fedora and Debian
  application dirs.
- The cache-vs-scan measurement recorded, and the cache kept only if it wins beyond noise.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

Ranking, UI, launching.
