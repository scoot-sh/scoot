---
title: "scaler_probe a_limit_just_above_the_full_draw_budget_still_draws: the daemon closes without replying under CPU load"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-30"
---

# scaler_probe a_limit_just_above_the_full_draw_budget_still_draws: the daemon closes without replying under CPU load

Filed 2026-10-29. Serves **daily-drive**: a flaky test erodes trust in the suite that guards the wallpaper daemon, and a daemon that exits mid-draw would be a product bug if the limit were not the test's own.

## The gap

`tests/scaler_probe.rs` `a_limit_just_above_the_full_draw_budget_still_draws` fails with `<filter>: scootbg: the daemon closed the connection without replying` (`scaler_probe.rs:287`, the `drawn.status.success()` assert). Measured on the 6-core aarch64 dev VM, scootbg code identical to main:

- full `-p scootbg -p scootbg-mem` runs, load average about 8 at the start of each: 3 of 10 (0.8 s, 9.9 s and 10.1 s into the test);
- the `scaler_probe` binary alone with 12 busy loops beside it: 2 of 8 (the run once failed on `nearest`, once on `catmull-rom`);
- the same binary on an unloaded VM: not seen.

The test sets `RLIMIT_AS` of the live daemon to `vm_size(pid) + LANCZOS_BUDGET + SHM_BYTES + HEADROOM`, read once beforehand. The lead is that under load the daemon maps more address space between that read and the draw (a worker thread's stack, a late allocation) than `HEADROOM` allows, and the daemon exits on its own limit. That is unconfirmed: the daemon's exit status and stderr were not captured.

## What to do

Capture the daemon's exit status and `scootbg.log` on that failure (the session keeps `daemon_log()`), then either widen the headroom with the measured growth or take `vm_size` again right before lowering the limit. If the daemon dies at a limit that normal use can reach, that is a product bug and gets its own regression test.

## Not in this ticket

Any other test.

## Resolution

Cause, captured 2026-10-30 on the dev VM (16 busy loops beside `-p scootbg`; 3 failures in 19 runs of the test alone): a test race, not a scaler probe bug. `set #102030` replies before the daemon's `scootbg-save` thread has even started. That thread maps its stack and a 64 MiB malloc arena when it runs, so the `VmSize` the test read was 11.4 MB (before the thread ran) in some runs and 78.6 MB (after) in others; the limits came out at 78,063,492 and 145,192,836 bytes. With the small one, the saver thread (spawned again by the draw's own `set`) and the decoding thread competed for the 64 MiB the limit left, and the loser aborted: `failed to allocate an alternative stack: Cannot allocate memory (os error 12)`, daemon exit 134, which is the "closed the connection without replying" the client saw. A second shape, a draw refused with `out of memory: cannot allocate 25320948 bytes for scaling` or `shared memory: Cannot allocate memory`, came from the same size moving between the read and the draw.

Fixed in the tests: `common::wait_for_saver_idle` waits until the daemon is down to one thread before `VmSize` is read (here and in `animated_oom.rs`, `draw_failed.rs` and the other `scaler_probe.rs` test, which read it the same way), and the margin test's failure now prints the daemon's exit status and log. 100 reads of the margin test's limit under the same load were all 145,172,356 bytes.
