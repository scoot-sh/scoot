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

## What to do

Reproduce under load (a full nextest run in a loop), keep the failure output, and find which wait or fixed delay is too short.

## Not in this ticket

Any other test.
