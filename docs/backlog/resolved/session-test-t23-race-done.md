---
title: "Session launcher harness T23 reads env-at-graphical before it exists"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-06"
---

# Session launcher harness T23 reads env-at-graphical before it exists

Filed 2026-11-06. Serves **daily-drive**: a flaky CI job trains people to
re-run past real failures.

## The gap

`scripts/scoot-session-test.sh` T23 waits for `start scoot-session.target` in
`calls.log` (`wait_log`), then reads `$HARNESS_STATE/env-at-graphical` at
once. On PR #955's CI the grep failed with `env-at-graphical: No such file or
directory` and then `FAIL: T23: the display was not the compositor's
wayland-100` (run 42256504577, job "Session launcher harness"). Every test up
to T22 passed, the PR touched no launcher code, and the re-run passed. The file
is written after the log line, so the read can run first.

## What to do

Wait for the file (a bounded loop like `wait_log`) before grepping it, or write
it before the call is logged. Check the other tests that read files from
`$HARNESS_STATE` right after a `wait_log` for the same ordering.

## Resolution

`wait_graphical` waits (bounded, ~10 s) for `env-at-graphical`, which the
fake `systemctl` now publishes last and atomically (tmp + `mv`), after the
`swayidle` state. T1, T3 and T23 call it after `wait_log`; they were the
three tests reading that state straight after the log line. T10 pre-writes
its files and never races. Proof: with a 0.3 s sleep injected into the fake
between the logged call and its effects, the old harness fails
deterministically (T1, then T3) and the fixed one passes 62/62.

## Not in this ticket

The launcher itself.
