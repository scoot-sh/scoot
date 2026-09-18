---
title: "Flake: parked-captures poll sees an extra frame_serial advance under full-suite load"
status: "open"
area: "rendering"
priority: "medium"
blocked: null
---

# Flake: parked-captures poll sees an extra frame_serial advance under full-suite load

Found by the PR #107 review (2026-09-18), not by that PR's diff: the
reviewer's 9 back-to-back full `nextest --workspace` runs on the dev VM
failed twice (~2/9) in
`screencopy::tests::parked_captures_on_two_sessions_are_all_delivered_by_one_confirm`
(`screencopy/tests.rs:777`): after `MapWindow`, a re-parked capture polls
`Ready` where the test asserts `Waiting` — `frame_serial` advanced one
extra time between park and poll.

Why it is not PR #107: the failing file/assertion is untouched by that
diff, and the diff is provably behavior-neutral in the harness (no `Tty`
exists without DRM, so `retry_render` stays false; the only headless
change is a never-taken branch). The test passes 15/15 in isolation on
the same HEAD; it fails only under full-suite parallel load — a
commit-coalescing/timing shape (a second damage event landing in a later
dispatch under contention), not a logic shape. The reviewer could not run
it on pre-PR `main` (read-only role), so pre-existence is established by
independence, not by bisection.

Disposition: the poll's "no extra serial advance between park and poll"
assumption needs pinning or a settle before the poll. Merged with
disclosure in PR #107 rather than blocking it or re-running till green.
