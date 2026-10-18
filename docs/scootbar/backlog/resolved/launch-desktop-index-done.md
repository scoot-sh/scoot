---
title: "scootlaunch: the desktop entry index, Exec expansion and `--list --json`"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-18"
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

## Resolution

Landed 2026-10-18 in PR #663 (`feat(scootlaunch): desktop entry index,
Exec expansion and --list --json`), eight commits on
`feat/scootlaunch-index`.

What shipped: `crates/scootlaunch` (binary, `#![forbid(unsafe_code)]`,
links only what std links) with the pure index modules — dirs/ids with
precedence and `Hidden` masking, the format and its escapes, localized
keys in the spec's order, `OnlyShowIn`/`NotShowIn` and `TryExec`,
actions, Exec quoting and field codes with the must-reject cases, and
the cache keyed by canonical path, inode and mtime — plus
`scootlaunch --list [--all] [--lenient] [--json]`. 118 unit tests, one
file per module. Fuzz targets for the parser (500,000 runs, 62 s) and
the expander (1,000,000 runs, 34 s), seeded from 10 NixOS store files,
7 Fedora 43 and 7 Debian stable application dirs plus hand-written
edge files and the spec's §7 examples, with three crash inputs kept as
regression seeds. CI classify output and job, the flake fileset
exclusion, the `/scootlaunch/` docs stub, and the stale
`scootbar/src/font.rs` comment path in `install.sh`.

The fuzzer found one real bug on the way: removed field codes could
promote a later `=` argument into the executable (`%f =foo` executed
`=foo`). The `=` check now runs on the expanded argv, pinned by a
unit test.

Cache-vs-scan at 2,048 files on the dev VM (release binary, commit
`3435a3c1b`): scan 224-243 ms, cached load 16-22 ms, cache file
747 KiB. Kept: ~12x, beyond noise. Functional rows: a profile-symlink
switch at frozen mtime 1 rescans, an in-place edit rescans, a corrupt
cache rescans with exit 0. Real data: 45 distro files, 0 strict-mode
drops; the system set indexes 7 of 8 files exactly as designed.
