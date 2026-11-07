---
title: "Pin idle-notify activity on the remaining gesture paths"
status: "open"
area: "testing"
priority: "low"
blocked: null
---

# Pin idle-notify activity on the remaining gesture paths

Filed 2026-11-07. Serves **daily-drive**: a gesture that does not wake an idle seat leaves the screen dimmed or locked mid-gesture.

## The gap

PR #967 pinned idle activity on `gesture_pinch_begin` and `gesture_hold_begin` only. Reviewer mutation check: removing `announce_activity()` from `gesture_pinch_update`, `gesture_pinch_end` or `gesture_hold_end` (`crates/scoot/src/compositor/gesture/client.rs`) leaves every test passing. The same holds for the swipe calls in `gesture.rs` (lines ~470, 497, 508).

## What to do

Extend the `Step::Watch` rig in `gesture/client/tests.rs` to cover update and end of pinch, end of hold, and swipe begin, update and end. Each must fail when its own `announce_activity()` is removed (prove by mutation).

## Not in this ticket

Behavior changes: only tests.
