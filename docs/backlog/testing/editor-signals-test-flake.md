---
title: "look_leaves_cli editor_signals_are_really_delivered flakes on macOS under load"
status: "open"
area: "testing"
priority: "low"
blocked: null
---

# look_leaves_cli editor_signals_are_really_delivered flakes on macOS under load

Filed 2026-11-07. Serves **daily-drive**: a flaky test trains people to
rerun CI and hides real failures.

## The gap

`editor_signals_are_really_delivered` (`crates/scootctl/tests/look_leaves_cli.rs:315`)
fails intermittently in full-suite runs on macOS. The PR #981 implementer saw it
fail on 3 of 8 runs on a clean `origin/main` worktree and on 2 to 3 of about 9
runs of the PR tree, with the panic "hx died by its signal". It passes when run
alone, and two reviewers' runs on the same code passed. It has not been seen in
Linux CI. Not yet reproduced or diagnosed; the likely cause is a signal race
under load, but that is a guess.

## What to do

Reproduce under load (`cargo test -p scootctl` in a loop beside a CPU hog),
read what the test waits on before it signals and checks, and fix the race, not
the symptom (no blanket retry or longer sleep without a reason). Record the
clean-run count the way `animated-oom-tight-limit-flake` does.

## Not in this ticket

The signal-scoping behavior itself (`editor_signals_reach_only_the_users_own_processes`).
