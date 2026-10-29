---
title: "scootbg's sway test helper has the same first-swaymsg start-up race"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-29"
---

# scootbg's sway test helper has the same first-swaymsg start-up race

Filed 2026-10-29. Serves **computer use** (the agent-facing suites stay trustworthy): a green CI is the floor under every change.

## The gap

`crates/scootbg/tests/common/mod.rs` `Session::sway` returns as soon as sway's Wayland and IPC sockets exist, and its tests then call `swaymsg` once with the tool's own 3 s limit. Sway binds the IPC socket during start-up and serves it only once its event loop runs, so on a starved runner the first `swaymsg` can time out with `Unable to receive IPC response`. That is the failure the scootbar suite had on `ubuntu-26.04` (`scootui::test_support`, fixed by waiting for `swaymsg -t get_version` to answer, with a deadline). The scootbg copy of the helper has no such wait. No scootbg failure has been seen yet.

## What to do

Add the same readiness wait to `Session::sway` (poll `swaymsg -s SOCKET -t get_version -r` until it succeeds or `PATIENCE` passes, panicking with sway's log), or share the scootui helper. Retry only that read-only query.

## Not in this ticket

Retrying commands (`create_output`, `output ... unplug`): a timed-out command may still run.

## Resolution (2026-10-29)

`Session::sway` now ends with `wait_for_sway_ipc`: it polls `swaymsg -s SOCKET -t get_version -r` until sway answers or `PATIENCE` passes, stops early and panics with sway's log if sway died. Only that read-only query is retried. scootbg does not depend on `scootui`, so the loop is a copy of `scootui::test_support`'s rather than a shared helper (a dev-dependency for 40 lines would not pay).

Evidence (dev VM, the tests pinned to one core under busy loops): sway's socket-to-first-answer window is 8-25 ms unloaded and 2.9-3.5 s with 100 loops (the `swaymsg` limit is 3 s). With the sway tests (`hotplug`, `color`, `image`) under that load, the `Unable to receive IPC response` failure occurred in 1 of 3 runs before (and 1 of 1 at 150 loops) and 0 of 4 after.
