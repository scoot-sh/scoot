---
title: "Pin nested host-pointer re-announce and the floating-drag end on pointer loss"
status: "open"
area: "testing"
priority: "low"
blocked: null
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
