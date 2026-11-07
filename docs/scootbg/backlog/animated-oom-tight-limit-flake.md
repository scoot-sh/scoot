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

## Update 2026-11-07: still not reproduced, 500 runs; the test now says where it failed

Measured on the 6-core aarch64 dev VM (nextest-built `animated_oom` binary, `SCOOTBG_REQUIRE_SCOOT=1`), none of these failed:

- 60 runs beside 16 busy loops; 60 runs with test and 8 busy loops all pinned to one CPU (`taskset -c 0`);
- 40 runs without `wait_for_saver_idle` (the pre-fix `VmSize` variance restored), idle, and 40 more beside 16 busy loops;
- 120 runs, 12 in parallel, beside 16 busy loops, without the wait;
- one full `-p scootbg -p scootbg-mem` nextest run beside 16 busy loops (690 passed, `animated_oom` among them);
- the margin swept down from 32 MiB to 16, 8, 4, 2 and 1 MiB (5 runs each) and, pinned to one CPU beside 6 busy loops, 30 runs each at 1 and 2 MiB: all pass. So the test does not sit near a margin cliff: both refusals reuse the saver's freed stack and arena, and the 1 GiB canvas fails at any margin;
- four overlapping `set`s (two with `--no-animate`), so the next decode thread starts as the previous one tears down, pinned to one CPU beside 6 busy loops, 30 runs each at 32, 4 and 1 MiB: all pass.

A refused `set` records nothing (`daemon::images`: a trial changes nothing until it lands), so only the decode thread runs in the lowered window, never a second saver thread.

The assertions now print the daemon's liveness or exit status, `VmSize` against the limit, its thread count and `scootbg.log` (`diagnosis` in `tests/animated_oom.rs`), which is what the next failure needs. This entry stays open until one does, or until a long enough clean record on CI says it was the already-fixed `VmSize` read.

A reviewer's hypothesis, not verified: if `std::thread::Builder::spawn` of the decode thread fails with `ENOMEM` under the lowered limit, `daemon/worker.rs` reports `cannot start a decoding thread: Cannot allocate memory (os error 12)` (exit 1, daemon still alive). The test's lowercase `cannot allocate` contains-check would miss that wording. The diagnosis output in `tests/animated_oom.rs` will show it on the next failure. If it recurs, the cheap hardening is to accept that refusal too, or to wait for the previous decode thread to end before the next `set`.
