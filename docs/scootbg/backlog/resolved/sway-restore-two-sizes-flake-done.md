---
title: "restore test a_restore_on_outputs_of_two_sizes_decodes_once_on_sway captured an all-black frame once"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-07"
---

# restore test a_restore_on_outputs_of_two_sizes_decodes_once_on_sway captured an all-black frame once

Filed 2026-10-21. Serves **daily-drive**: a flaky test erodes trust in the suite that guards the wallpaper daemon.

## The gap

The screenshot helper takes one shot after a fixed wait, so on a slow runner it can capture before sway presents the restored frame.

## What to do

Poll the screenshot until the flat color shows or a deadline passes (in `tests/common/shots.rs`), so a slow runner waits and a real failure still fails.

## Not in this ticket

Anything in scootbg itself: the daemon painted correctly on the re-run.

## Sighting

First sighting 2026-10-21 on PR #701 (a site-only vite bump, so no scootbg
change), job "scootbg on headless scoot", run 39667263669: the screenshot
helper (`tests/common/shots.rs:81`) saw all nine samples `[0, 0, 0]` ("not one
flat color") on `a_restore_on_outputs_of_two_sizes_decodes_once_on_sway`; the
other 81 tests passed and the re-run of the job was green. Reads as a capture
taken before sway presented the restored frame. Next step if it recurs: poll
the screenshot until the flat color shows or a deadline passes, instead of one
shot after a fixed wait.

## Resolution

Cause: a test that waited on the wrong signal, not a scootbg bug and not a slow runner. The ticket's premise ("one shot after a fixed wait") was off: the test already waited, with `query_until`, for both outputs to *show* the image. But `query`'s `shows` is what the daemon last committed, not what the compositor has read. The `set` replies the other sway tests wait on are different: the daemon sends one only after a `wl_display.sync` that follows its commits has come back (`crate::waiters`), so sway has them. After a restore there is no reply to wait on, only `query`, and a `query` reply is written inside the poll turn that serviced it while the Wayland flush for that turn's commits comes at the top of the next one. Preempt the daemon there, or sway on its side, and the screencopy that follows presents sway's black background. The CI failure (run 39667263669, nine `[0, 0, 0]` samples, the failed test ran 3.479 s, no longer than the 4.642 s of the same test passing in the job's re-run) is that frame; nothing says the runner was slow, only that a short window was hit once.

Not reproduced by load alone: 60 runs of the unfixed test on the 6-core aarch64 dev VM (30 beside 16 busy loops, 30 pinned to one CPU with 4 busy loops) all passed; an earlier pinned run with 8 busy loops was stopped before it reported a count, so it is not counted. It reproduces deterministically with the window widened: a local-only patch to `Daemon::flush_wayland` (not committed) that sleeps `SCOOTBG_TEST_FLUSH_DELAY_MS` before every flush. With the unfixed test that fails with the CI signature at 300 ms (0 of 5 pass, 11.06 s, nine `[0, 0, 0]` samples) and mostly at 100 ms (1 of 5 pass).

Fixed in the test: after `shows`, `a_restore_on_outputs_of_two_sizes_decodes_once_on_sway` sends `set '#ff0000' --workspace fence`. A mapping for a workspace that is not active changes nothing on screen (the test checks `query` again afterwards), and its reply waits for the sync, so the screencopy follows sway reading the restore's commits. Red, not the blue under test, so a mapping that did land would fail the check instead of passing it. With the fix: 5 of 5 at 300 ms and 5 of 5 at 100 ms under the injection, and no failure in 90 runs without it (30 idle, 30 beside 16 busy loops, 30 pinned with 4 busy loops). The failure message now names the output.

Left alone: the other `shows` followed by `shot` sites in `restore.rs` take the same short window, but they read scoot's framebuffer through its IPC rather than sway's, and the state-file checks in some (a restore "writes nothing") rule out a mapping as a fence. If one of them flakes, `query` could flush before it replies, which would close the window for every caller.
