---
title: "Help follow-ups: one suggest(), build-gated push wording, live llms.txt URL"
status: "open"
area: "ipc"
priority: "low"
blocked: null
---

# Help follow-ups: one suggest(), build-gated push wording, live llms.txt URL

Filed 2026-10-05 from PR #436's review (non-blocking N1-N3). Serves
**computer use** (help an agent reads first) and lightness.

## The gap

- `suggest()`/`levenshtein()` is copied into `scootctl`, `scootbar` and
  `scootbg` `help.rs` (~40 lines each; `scoot` reuses scootctl's). One copy
  in `scoot-ipc` (already a dependency of all three) would shave part of
  scootctl's +4.2% `.text`.
- A scootbar build without the `push` feature still describes push modules
  in `msg help set` (`crates/scootbar/src/help.rs`, the `set` row), while the
  same file gates examples per build.
- `crates/scootctl/src/help.rs` and `docs/cli-help.md` say `/llms.txt` is
  published "once the docs site lands". It is live now at
  `https://www.scoot.sh/llms.txt`: point the
  SEE ALSO lines at the real URL, from one constant so the domain move is one
  edit.

## What to do

All three, with the single-source tests extended (a no-`push` build's help
names no push module); re-measure `.text` per binary.
