---
title: "scootbg-mem: closing_the_fd_keeps_the_pages test races on fd-number reuse"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-03"
---

# scootbg-mem: closing_the_fd_keeps_the_pages test races on fd-number reuse

Filed 2026-09-29 (PR #332 review). Serves CI reliability.

## The gap

`scootbg-mem shm::tests::closing_the_fd_keeps_the_pages_and_frees_the_descriptor` closes its fd and then asserts `/proc/self/fd/N` is gone. Under `cargo test`, another test thread can reuse fd N in between, so the assertion fails ("the memfd is still open"). Reproduced 1 in 60 runs of `cargo test -p scootbg-mem --lib`; it failed the `scootbg (wallpaper daemon)` CI job on PR #332, which did not touch scootbg-mem.

## What to do

Assert on identity, not on the fd number: compare the fd's `fstat` inode/dev (or the `/proc/self/fd/N` link target) with the memfd's, so a reused number does not read as "still open".

## Not in this ticket

Any product change: this is test hygiene only.

## Resolution (2026-10-03, already landed)

No code change needed: PR #331 (`21561385`, merged 2026-09-29 12:46 -0400) fixed this ~4 minutes before the ticket was filed (`27b4876b`). `crates/scootbg-mem/src/shm/tests.rs:105-121` already fstats the memfd before `close_fd` and compares dev+ino, treating a reused fd number as a pass. Verified with 46 green `cargo test -p scootbg-mem` suite runs across thread counts plus mutation checks (no-op `close_fd` fails, leaked-fd fails).
