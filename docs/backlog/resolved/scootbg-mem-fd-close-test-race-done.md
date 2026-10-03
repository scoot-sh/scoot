---
title: "scootbg-mem: closing_the_fd_keeps_the_pages test races on fd-number reuse"
status: "open"
area: "testing"
priority: "medium"
blocked: null
---

# scootbg-mem: closing_the_fd_keeps_the_pages test races on fd-number reuse

Filed 2026-09-29 (PR #332 review). Serves CI reliability.

## The gap

`scootbg-mem shm::tests::closing_the_fd_keeps_the_pages_and_frees_the_descriptor` closes its fd and then asserts `/proc/self/fd/N` is gone. Under `cargo test`, another test thread can reuse fd N in between, so the assertion fails ("the memfd is still open"). Reproduced 1 in 60 runs of `cargo test -p scootbg-mem --lib`; it failed the `scootbg (wallpaper daemon)` CI job on PR #332, which did not touch scootbg-mem.

## What to do

Assert on identity, not on the fd number: compare the fd's `fstat` inode/dev (or the `/proc/self/fd/N` link target) with the memfd's, so a reused number does not read as "still open".

## Not in this ticket

Any product change: this is test hygiene only.
