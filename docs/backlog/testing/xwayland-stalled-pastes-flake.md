---
title: "xwayland clipboard stalled_pastes_do_not_hold_the_bound_for_good fails once under cargo test"
status: "open"
area: "testing"
priority: "low"
blocked: null
---

# xwayland clipboard stalled_pastes_do_not_hold_the_bound_for_good fails once under cargo test

Filed 2026-10-28. Serves **daily-drive**: a test that fails at random hides real regressions.

## The gap

On PR #785 (head `6b67142`, a comments-only delta from a green head) the
`Linux (xwayland)` job passed all 2908 nextest tests, then failed in the
`cargo test` step: `compositor::xwayland::tests::clipboard::stalled_pastes_do_not_hold_the_bound_for_good`
panicked at `crates/scoot/src/compositor/xwayland/tests/clipboard.rs:919`
("the bound was still full of transfers whose readers had gone"). Re-running
the failed job on the same head passed. The PR touched no XWayland code, and
`cargo test` runs tests concurrently in one process, which nextest does not.

The test drops its readers and expects the freed slot to be usable on the next
paste. A freed slot that is not yet reclaimed when the next paste arrives
fails exactly this assertion, so a timing or ordering race between the reader
close and the transfer bookkeeping is the first suspect. Not yet shown to be a
test race and not a product bug (a paste refused after its readers went away).

## What to do

Reproduce under `cargo test -p scoot` with the load CI has (loop it,
`--test-threads` high). If the slot is reclaimed asynchronously, make the test
drain until the reclaim has run instead of asserting right after the drop; if
the compositor can refuse a paste while a dead reader's slot is still held,
fix that and keep the test strict.

## Not in this ticket

The other transfer-bound tests in the same file, which passed.
