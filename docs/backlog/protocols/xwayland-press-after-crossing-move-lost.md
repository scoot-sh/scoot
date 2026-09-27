---
title: "XWayland: a press batched with a move onto another X window reached no X window (harness)"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# XWayland: a press batched with a move onto another X window reached no X window

Filed 2026-09-27, found in passing while measuring
[the X drag first-motion race](../resolved/xwayland-x-drag-first-motion-race-done.md).
**Observed in the test harness only; not investigated, not reproduced
live.** Serves computer use first (an agent pipelines `pointer move` and
`pointer button`) and daily use second (a fast flick-and-click, where
libinput can deliver the motion and the button in one batch).

## What was seen

In `compositor/xwayland/tests/`'s live harness, with the pointer moving
from one X window onto another X window and a press in the same batch --
`State::pointer_move` then `State::pointer_button` with no dispatch of the
event loop between them -- XWayland delivered the press to **no** X
window: neither the window the pointer left nor the one it entered saw a
`ButtonPress`. The measurement (`first_motion_race.rs`) worked around it
by moving first and letting the move settle, as a real pointer would, and
nothing about the count was recorded.

## Why it is filed under protocols, not testing

It may be a harness artifact (the harness calls `State` directly, not
through the IPC listener or libinput's frame boundaries), but nothing yet
rules out a product bug: scoot's `pointer move` and `pointer button` IPC
requests pipelined on one connection are served in one wakeup with one
flush, which is the same shape, and so is a libinput frame carrying both a
motion and a button. If XWayland only routes a press after it has
processed the `wl_pointer.enter` for the new surface in some later
dispatch, a click landing on a different X window than the last one could
be dropped.

## To do

1. Reproduce outside the harness: two X windows that report presses
   (`xev`, or an x11rb window selecting `ButtonPress`), then a `scoot msg`
   pointer move onto the second window and a button press pipelined on
   one connection. Count presses seen per 20.
2. If it reproduces, find which side drops it: the `wl_pointer`
   enter/button order and frames scoot sends XWayland for the batch
   (`WAYLAND_DEBUG=server` on scoot), against what XWayland's
   `xwl_seat` does with a button that arrives in the same frame as an
   enter.
3. If it only happens in the harness, move this to `testing/` and note it
   next to the harness's helpers so the next test does not trip on it.
