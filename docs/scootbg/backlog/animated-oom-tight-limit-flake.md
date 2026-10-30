---
title: "animated_oom animation_canvases_refuse_under_a_tight_limit: failed once in a full-suite run (not reproduced)"
status: "open"
area: "scootbg"
priority: "low"
blocked: null
---

# animated_oom animation_canvases_refuse_under_a_tight_limit: failed once in a full-suite run (not reproduced)

Filed 2026-10-30. Serves **daily-drive**: a flaky test erodes trust in the suite that guards the wallpaper daemon.

## The gap

`tests/animated_oom.rs` `animation_canvases_refuse_under_a_tight_limit` failed once as test 2 of 663 in a fail-fast `-p scootbg -p scootbg-mem` nextest run on the 6-core aarch64 dev VM under load (load average about 8). The output was not kept, so the assertion that tripped is unknown.

Not reproduced: 0 failures in 37 runs of the test alone with 16 busy loops beside it, and none in 7 full-suite runs. What the runs did show is the same instability its neighbour had (`resolved/scaler-probe-margin-daemon-exit-flake-done.md`): the test read `VmSize` while the daemon's saver thread had not started, in 10 of 37 runs (limit 44,658,688 against 111,788,032 once the thread had mapped its stack and arena). The wait for the saver thread that fixed the neighbour is now in this test too, which removes that variance but is not shown to be the cause of the one failure.

## What to do

If it fails again, keep the nextest output: the `animated_oom` assertions print the `set` stderr and the daemon's liveness, which says whether the daemon died (an abort at the limit, as the neighbour's was) or refused differently. A daemon that dies at a limit that normal use can reach is a product bug and gets its own regression test.

## Not in this ticket

Any other test.
