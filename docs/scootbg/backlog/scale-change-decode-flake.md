---
title: "scale test a_scale_change_while_an_image_decodes_draws_at_the_new_scale failed once in a full nextest run on the dev VM"
status: "open"
area: "scootbg"
priority: "low"
blocked: null
---

# scale test a_scale_change_while_an_image_decodes_draws_at_the_new_scale failed once in a full nextest run on the dev VM

Filed 2026-10-28. Serves **daily-drive**: a flaky test erodes trust in the suite that guards the wallpaper daemon.

## The gap

`cargo nextest run -p scootbg -p scootbg-mem -p scootlaunch` on the 6-core aarch64 dev VM failed `scootbg::scale a_scale_change_while_an_image_decodes_draws_at_the_new_scale` once (11 s, test 97 of 983, many tests running at once). The next full run passed 983/983, and the test passed 20 of 20 run alone. The failure output was not captured, so the cause is unknown; load-dependent timing is the lead.

## Captured (2026-10-29)

Seen again, 1 of 10 full `-p scootbg -p scootbg-mem` runs on the idle 6-core dev VM (test 95 of 661, 11.5 s), with the output kept. The test's own line reads `rescaled 1.163060458s after the request, reply after 1.164242625s`: the reply came 1 ms after the reload returned, and the assertion at `scale.rs:331` found a `create_buffer(..., 0, 1600, 1000, 6400, 1)`, the old scale's buffer. So the 4000x2500 decode finished and was drawn at scale 1 before the reload's scale change reached the daemon, and the reload took 1.16 s on a machine running six tests at once. The test assumes the scale change lands while the decode is still in flight; a decode that completes first leaves no mid-decode change to test. The lead is that race, not a missing wait for the draw. The same run's other failure was a different test (`rotation`, fixed in the PR that changes `tests/rotation.rs`).

## What to do

Reproduce under load (a full nextest run in a loop), keep the failure output, and find which wait or fixed delay is too short.

## Not in this ticket

Any other test.
