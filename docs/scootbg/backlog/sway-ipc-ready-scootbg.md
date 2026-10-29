---
title: "scootbg's sway test helper has the same first-swaymsg start-up race"
status: "open"
area: "scootbg"
priority: "medium"
blocked: null
---

# scootbg's sway test helper has the same first-swaymsg start-up race

Filed 2026-10-29. Serves **computer use** (the agent-facing suites stay trustworthy): a green CI is the floor under every change.

## The gap

`crates/scootbg/tests/common/mod.rs` `Session::sway` returns as soon as sway's Wayland and IPC sockets exist, and its tests then call `swaymsg` once with the tool's own 3 s limit. Sway binds the IPC socket during start-up and serves it only once its event loop runs, so on a starved runner the first `swaymsg` can time out with `Unable to receive IPC response`. That is the failure the scootbar suite had on `ubuntu-26.04` (`scootui::test_support`, fixed by waiting for `swaymsg -t get_version` to answer, with a deadline). The scootbg copy of the helper has no such wait. No scootbg failure has been seen yet.

## What to do

Add the same readiness wait to `Session::sway` (poll `swaymsg -s SOCKET -t get_version -r` until it succeeds or `PATIENCE` passes, panicking with sway's log), or share the scootui helper. Retry only that read-only query.

## Not in this ticket

Retrying commands (`create_output`, `output ... unplug`): a timed-out command may still run.
