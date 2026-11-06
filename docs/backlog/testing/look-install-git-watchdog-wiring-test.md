---
title: "Pin that the git install registers its process group with the watchdog"
status: "open"
area: "testing"
priority: "low"
blocked: null
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
