---
title: "Session launcher harness T23 reads env-at-graphical before it exists"
status: "open"
area: "testing"
priority: "low"
blocked: null
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

## Not in this ticket

The launcher itself.
