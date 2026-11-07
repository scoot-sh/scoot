---
title: "Pin idle-notify activity on the pinch and hold paths"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-06"
---

# Pin idle-notify activity on the pinch and hold paths

Filed 2026-11-06 from the review of PR #946. Serves **daily-drive**: a
pinch or hold on the touchpad must count as activity, so the screen does not
blank or lock under someone who is using it.

## The gap

With `[gestures] client_gestures = true`, the pinch and hold arms in
`crates/scoot/src/tty/mod.rs` call `announce_activity()` (idle-notify) and
`note_pointer_activity()` (cursor reshow). The test
`with_the_global_a_pinch_or_hold_is_pointer_activity` in
`compositor/gesture/client/tests.rs` asserts only the second. The reviewer
removed `announce_activity()` from `gesture_hold_begin` and every test still
passed.

## What to do

Assert the idle-notify side beside the cursor-deadline assertion, for pinch
begin and hold begin, and check that it fails with the call removed.

## Not in this ticket

The default-off path: it is already pinned by
`the_global_is_absent_by_default`.

## Resolution

2026-11-06. `a_pinch_begin_is_idle_activity` and `a_hold_begin_is_idle_activity`
in `compositor/gesture/client/tests.rs` bind `ext_idle_notifier_v1` in the
gesture test client, wait for `idled`, drive the gesture, and require a
`resumed`. Removing `announce_activity()` from `gesture_pinch_begin` or
`gesture_hold_begin` makes the matching test fail.
