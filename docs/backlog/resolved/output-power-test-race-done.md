---
title: "output_power test set_mode_off_reaches_every_holder races under load"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-06"
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

## Resolution (2026-10-07, PR #488)

Test-quality fix, product code untouched. `wait_for_event` now waits on a
10 s wall-clock deadline (`WAIT_FOR_EVENT`) instead of 50 roundtrips, and
the second-output registry wait in
`removing_the_output_fails_its_controls_and_forgets_the_state` got the
same treatment (identical fixed-budget shape). No sleeps. Established
test-side first: `broadcast_mode` reaches every holder and
`get_output_power` reflects the off state, so the server side was never
suspect. Other waits in the file audited: the repeat-set drain is a fixed
settle, not an event wait; the two session-lock waits already advance on
wall-clock time (20 ms sleeps x 100). `gamma_control` and `selection`
keep their own fixed-budget helpers, but every test there is
single-client, so the cross-thread ordering race cannot arise; left alone.

Proof on the Asahi M2 under 8 busy loops, same tree: before, 57/200
(direct binary), 47/200 (`cargo test --exact`, one process like CI),
46/200 (nextest) fail at `tests.rs:392` with "the second client: the
compositor never sent the off mode"; after, 200/200 in all three
harnesses, zero failures. Full verification on the same tree:
`cargo nextest run --workspace` 4443 passed / 36 skipped / 0 failed,
`cargo test -p scoot` 2188 + 4 passed / 0 failed, clippy and fmt clean.
