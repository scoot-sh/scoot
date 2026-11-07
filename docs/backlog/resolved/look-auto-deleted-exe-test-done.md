---
title: "Pin the look auto vanished-executable refusal"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-07"
---

# Pin the look auto vanished-executable refusal

Filed 2026-11-06 from the review of PR #959. Serves **daily-drive**: a timer
that points at a binary that was replaced must refuse at install, not fire
into nothing.

## The gap

`run_on` in `crates/scootctl/src/look/auto.rs` refuses with "the scoot binary
... no longer exists (replaced while running?)" when the baked executable path
does not exist (`current_exe()` reports `... (deleted)` after an upgrade
replaced the running binary). The check is inline, so no unit test reaches
it: the reviewer disabled `!exe.exists()` and all 35 `look::auto` tests still
passed.

## What to do

Extract the check as a small pure function over a path and test a path that
does not exist and a path with a `(deleted)` suffix. Show the test failing
with the refusal removed.

## Not in this ticket

The `--sun` and `--follow-portal` triggers (see `look-auto-sun.md` and
`look-auto-follow-portal-done.md`).

## RESOLVED

The check moved out of `run_on` into `unit_exe` (`look/auto.rs`), which
takes the running binary and `PATH`, applies `stable_exe` and refuses a
file that does not exist. `run_on` needs its result as the unit's
executable, so the wiring cannot be dropped without losing the path.
Three tests in `auto/tests.rs`: an existing file is accepted; a missing one
is refused with the re-run advice; a `scoot (deleted)` path (what
`current_exe()` reads on Linux after an upgrade) is refused. Mutation:
`if false` fails the two refusal tests; `if exe.exists()` fails all three.
No behavior change, so no real bug found.
