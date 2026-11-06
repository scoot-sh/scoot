---
title: "Pin the look auto vanished-executable refusal"
status: "open"
area: "testing"
priority: "low"
blocked: null
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
`look-auto-follow-portal.md`).
