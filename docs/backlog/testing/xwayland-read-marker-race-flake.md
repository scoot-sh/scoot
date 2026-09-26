---
title: "Flake: xwayland test `read_marker` polls a file that exists-but-empty between redirect setup and echo"
status: "open"
area: "testing"
priority: "low"
blocked: null
---

# Flake: `read_marker` reads empty between redirect setup and echo

Filed 2026-09-26 from the PR #257 review (independent re-derivation, not
the implementer's report): `read_marker` (`tests/mod.rs:81`, Phase-1-era,
untouched by that diff) polls `read_to_string`, but `sh -c "echo … >
marker"` creates the file at redirect setup *before* echo writes — a poll
landing in that gap reads `""` and the `:9` assertion fails. Passed alone
and on full re-run; same load-sensitive class plausibly explains #257's
reported one-window dnd full-run flake (at n=1 the cap is one `HashMap::get`,
off the dnd timing path).

Fix shape (suggestion, not prescription): wait-for-nonempty or
write-then-rename in the helper. Pinned by a stress loop, not a single
green run.
