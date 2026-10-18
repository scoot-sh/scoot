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

## Resolution

Landed 2026-10-18 in PR #664 (`feat(scootlaunch): fuzzy matching and
frecency, --query --json`), seven commits on `feat/scootlaunch-rank`.

What shipped: `src/rank/fuzzy.rs` (case-folded dynamic program with
start/word/consecutive bonuses and gap penalties, field weights
`Name` 1.0, `GenericName` 0.6, `Keywords` 0.5, Exec basename 0.4, and
the 64-bit character-mask pre-filter), `src/rank/frecency.rs` (the
history file: at most 512 entries and 32 KiB, tmp-file-plus-rename
atomicity, malformed lines skipped, oversized files ignored, no
panic on hostile bytes, safe under concurrent writers), `src/rank.rs`
(one row per entry plus one per desktop action, a `Ranker` with
reused buffers, the combined ordering with the frecency bonus capped
at a quarter of a perfect match), `scootlaunch --query TEXT
[--all] [--lenient] [--json]` (each entry once at its best row's
position, same JSON schema in ranked order), and `Env.xdg_state_home`
(`$XDG_STATE_HOME`, default `~/.local/state`). 160 unit tests: the
ordering pins (a prefix beats a scattered match, a habit never beats
a much better match and does break a close one, Unicode and case
folding, empty/one-char/0/1/3000-entry edges), a `count_allocations
== 0` warm-query test (Linux-only, like the allocator it counts
through), and a latency report test. Fuzz targets for the matcher
(500,000 runs, 8 s) and the history reader (250,000 runs, ~1 s) with
hand-written seeds, added to the crate's fuzz crate and the CI
budget. The `count_allocations` dev-dependency is target-gated
because `scootbg-mem` does not build on macOS. Site updates in the
same PR (`scootlaunch` overview, CLI reference, ranking and history
section, `--query --json` schema note). The launch-spawn ticket calls
`Frecency::record` after a launch; it lands here with its bounds and
atomicity pinned.

Perf on the M2 (idle, release, commit `c87e2efd3`, 3,000 synthetic
entries): worst keystroke 0.64 ms (empty query, full sort), single
char 0.46 ms, 13-char match-many query 0.46 ms, prefix queries
27-37 us. The first cut exceeded the 1 ms cap (2.7 ms match-many);
fields now store codepoints, exact prefixes answer directly, and
score ties break by row index.

Mutations reverted in a scratch worktree, each failing its pin:
`START` to 0 (consecutive unit test), the bonus cap removed (the
habit test flips), `Name`/`Keywords` weights swapped (the keyword
test flips).
