---
title: "`zwp_pointer_gestures_v1`: hand swipes, pinches and holds to clients"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# `zwp_pointer_gestures_v1`: hand swipes, pinches and holds to clients

Filed 2026-10-30 (touchpad swipe gestures, #820). Serves **daily-drive**: a
browser's pinch-to-zoom and three-finger history swipe, a map's pinch, a
viewer's rotate all arrive through this protocol on other compositors.

## The gap

scoot never advertises `zwp_pointer_gestures_v1`, and the libinput dispatcher
(`compositor/tty/mod.rs`) used to end in `_ => {}` for every `Gesture*` event.
So no client can bind the swipe, pinch or hold objects, and every multi-finger
gesture was dropped.

The swipe half is wired up as of the gestures change: `gesture.rs` decides at
`begin` whether a swipe is scoot's (`[gestures] swipe_fingers`) or the
client's, and forwards the client's through `pointer.gesture_swipe_begin`,
`_update` and `_end` (all three, whole, with a cancelled `end` if scoot ever
abandons one). Today those calls reach nobody: with no global there is no
bound object to send to. Pinch and hold are still dropped at the libinput
arm.

## What to do

1. `PointerGesturesState::new::<State>(&dh)` and `delegate_pointer_gestures!`
   (the pinned fork's Smithay carries the module). A global that clients bind
   changes what GTK4, Firefox and Chromium do with a touchpad, so check each
   against a real pad before turning it on by default.
2. Add the libinput arms for `GesturePinch*` and `GestureHold*` beside the
   swipe ones, with the same begin-decides-and-end-matches rule (see the
   module doc of `gesture.rs`): no client gets an `update` or `end` for a
   `begin` it did not get.
3. A swipe forwarded to a client whose pointer focus changes mid-swipe
   (the pointer moved to another surface, a window closed) currently sends
   the `end` to the new focus. Decide whether that needs a cancelled `end` to
   the old focus first; Smithay's `PointerHandle` does not do it for us.
4. `--nested` could bind the host's `zwp_pointer_gestures_v1` and route its
   swipes through the same `gesture_swipe_*` entry points, which would make
   `[gestures]` work in a nested session.

## Not in this ticket

Touch-screen (multi-touch) gestures, and `scoot msg` injection of gestures.
