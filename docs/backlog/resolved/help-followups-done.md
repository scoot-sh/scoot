---
title: "Help follow-ups: one suggest(), build-gated push wording, live llms.txt URL"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-06"
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

## Resolution (2026-10-06, PR #478)

All three landed, verified on the Asahi M2 (release `.text`:
scoot +0.13%, scootbar +0.06%, scootbg +0.06%; file sizes byte-identical;
`nextest --workspace` 4373 passed, `cargo test -p scoot` pass, clippy/fmt
clean, `nix build .#docs-site` pass):

- `suggest()`/`levenshtein()` live once in `scoot-ipc` (`help.rs`, with
  `DOCS_URL` and `docs_tail()`); the `scootctl`/`scootbar`/`scootbg`
  copies are deleted. Re-check notes: #464's binary removal left the
  crate (and all three copies) in place, but "already a dependency of
  all three" no longer holds (scootbar dev-only, scootbg absent), so
  this adds both edges plus a CI classify arm (`scoot-ipc` -> scootbg),
  revisiting `crate-and-daemon-done.md`'s self-containment call (that
  weighed protocol framing; this shares the guesser with drift tests
  pinning it). The shared copy saved no `.text` (monomorphization);
  the win is source dedup plus single-sourcing.
- A no-`push` scootbar build names no push module: the `set` row (text
  and JSON), the `msg` set section and the `set` examples follow the
  feature. Proven by revert (both new gate tests fail) and restore
  (430 pass in `--no-default-features --bin`).
- Every SEE ALSO line ends on the live `https://www.scoot.sh/llms.txt`,
  rendered from the one constant; `docs/cli-help.md` drops "once the
  docs site lands".
