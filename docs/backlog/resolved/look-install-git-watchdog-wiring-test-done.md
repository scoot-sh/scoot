---
title: "Pin that the git install registers its process group with the watchdog"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-07"
---

# Pin that the git install registers its process group with the watchdog

Filed 2026-11-06 from the review of PR #952. Serves **daily-drive**: a
`scoot look install GIT-URL` that is killed must not leave git or ssh running.

## The gap

`crates/scootctl/src/look/install/git.rs` registers git's process group with
the watchdog right after spawn (`register_group(child.id())`). The reviewer
deleted that call and all 50 `look::install::git` tests still passed. The
live `kill -9` check showed the wiring works today, but a regression would go
unnoticed. `a_killed_scoot_leaves_no_git_and_no_temp_dir` is the end-to-end
test meant to cover it; check why it did not fail.

## What to do

Make a test fail when the registration is removed, and show the failure
with the call deleted. Also consider the unregistered window between `spawn`
and `register_group`, where a SIGKILL of scoot leaves that git unregistered.

## Not in this ticket

The empty-repository suffix keying on git's English error text.

## Resolution (2026-11-07)

The end-to-end test did catch the deletion: with `register_group(child.id())`
removed, `a_killed_scoot_leaves_no_git_and_no_temp_dir` fails ("helper N gone:
false") on macOS and on the Linux dev VM. The review's "all 50 tests passed"
ran the `look::install::git` unit filter, which does not include the
`tests/look_install_git_cli.rs` integration binary. The gap was in the unit
suite, and an end-to-end test takes 20 s to fail.

Added `the_runner_registers_gits_group_with_the_watchdog_then_clears_it`
(`git/tests.rs`): a thread-local log of `Scratch::register_group` calls
(`git/scratch.rs`, test-only, per thread so parallel tests stay apart) shows
the stand-in git's own pid registered, then 0. Deleting the register call
fails it; deleting the clearing call fails it.

The window between `spawn` and `register_group` stays open on purpose, and a
comment at the call says so: closing it needs an async-signal-safe write from
the forked child, for a microseconds-wide window whose only cost is a git that
fails on its removed directory.
