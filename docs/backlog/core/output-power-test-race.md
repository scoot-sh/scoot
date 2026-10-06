---
title: "output_power test set_mode_off_reaches_every_holder races under load"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# output_power test set_mode_off_reaches_every_holder races under load

Filed 2026-10-06 from the #482 round-3 review. A test-quality bug, not a
product bug. It still matters: CI runs `cargo test` with tests in one
process, so the race shows up as a red CI run on an unrelated PR.

## The gap

`output_power::tests::set_mode_off_reaches_every_holder` fails at
`tests.rs:392`. Rates seen:

| Where | Failures |
| --- | --- |
| `origin/main`, fix-report loops | 5 of 270 runs |
| the #482 branch | 13 of 270 runs |
| the same test binary on the M2, idle | 0 of 200 |
| the same test binary on the M2, under 8 busy loops | 41 of 200 |

The cause is in the test itself. `wait_for_event` gives each client a
fixed budget of 50 roundtrips, and the two client threads aren't
synchronized. Under CPU contention, the second client uses up its budget
before the first one has sent `Off`.

## What to do

- Replace the roundtrip budget with a wall-clock deadline (a few seconds),
  or have the second client wait until the first has sent `Off`.
- Prove the fix with the same load harness: 0 failures in 200 runs under 8
  busy loops, where the old test fails at about 20%.
- Check the other `wait_for_event` callers for the same fixed-budget
  pattern.
