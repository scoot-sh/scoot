---
title: "xwayland clipboard stalled_pastes_do_not_hold_the_bound_for_good fails once under cargo test"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-28"
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

## Resolution

A test race, not a product bug. The compositor frees a stalled paste's slot
by polling its pipe for hang-up (`reader_gone` in the pinned Smithay fork),
and a pipe only hangs up once every process holding the read end has closed
it. `cargo test` runs the tests as threads of one process, so another test
forking a child (an Xwayland starting) copies this test's read ends, and the
child holds them from `fork` until `exec`. A paste arriving in that gap is
refused because the reader really has not gone. nextest gives each test its
own process, so it never sees this.

Reproduced on the dev VM by forking children in a loop alongside the test
(the fork-to-exec gap widened to 20 ms): 14 of 40 runs failed at the same
assertion. Plain CPU load alone did not reproduce it (100 of 100 passed with
12 busy loops, and 100 of 100 with a fork loop and no widening). The test
now keeps pasting until one is admitted, for up to 5 s, and fails with the
same message if none is; under the same widened fork loop 100 of 100 pass,
and with the readers never dropped it still fails after the 5 s.
