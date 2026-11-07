---
title: "Pin nested host-pointer re-announce and the floating-drag end on pointer loss"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-07"
---

# Pin nested host-pointer re-announce and the floating-drag end on pointer loss

Filed 2026-11-06 from the review of PR #962. Serves **daily-drive**: a host
that re-sends `wl_seat.capabilities` (common) must not cost a nested session
its pointer.

## The gap

The nested seat handler in `crates/scoot/src/compositor/nested_dispatch.rs`
releases the host pointer (and its gesture objects) only when the host
withdraws the pointer capability. Two behaviors have no test:

- A `Capabilities` event with the pointer still set must leave the host
  pointer and its gesture objects alone. The reviewer changed the second
  branch to `else if host.pointer().is_some()`, which tears the pointer down on
  every re-announce, and all 83 `nested*` tests still passed.
- Losing the pointer ends a floating-window drag (`end_floating_grab` and
  `settle_floating_grab` in the `!wants_pointer` branch).

## What to do

Add a test per case to `nested/gestures/tests.rs` or a sibling and show the
re-announce mutation failing. Also correct two small statements: the
`gesture/client.rs` module doc and the PR text say a `begin` over an open one
is closed by the entry point, which holds for swipe only (`gesture_pinch_begin`
and `gesture_hold_begin` forward the second begin unclosed, as on `--tty`); and
`site/.../backends.md` could say that a host seat losing its pointer now
releases it and ends a floating drag.

## Not in this ticket

The real-touchpad check (item 1 of `pointer-gestures-followups.md`).

## Resolution

2026-11-07. Two rig tests in `nested/gestures/tests.rs`:
`a_capabilities_re_announce_with_the_pointer_still_there_leaves_it_and_its_gestures_alone`
(the host adds a touch device, which re-sends the capabilities with the
pointer still set; the host pointer object, the gesture objects and a swipe
open across it must all survive) fails with the second branch changed to
`else if host.pointer().is_some()`, and
`the_host_seat_losing_its_pointer_ends_a_floating_drag` (a Super-drag of a
floating window, then the host seat removes its pointer) fails with
`end_floating_grab`/`settle_floating_grab` deleted from that branch.

One premise in the ticket was off: `gesture_pinch_begin` and
`gesture_hold_begin` do forward a second begin unclosed, but Smithay's gesture
object closes the old one `cancelled` (pinned by
`a_begin_over_an_open_gesture_closes_the_old_one_cancelled`), so a client sees
the same thing for all three. The wrong sentence was in `nested/gestures.rs`
(not `gesture/client.rs`, which is accurate) and is corrected there.
