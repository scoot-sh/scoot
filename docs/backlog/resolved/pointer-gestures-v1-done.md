---
title: "`zwp_pointer_gestures_v1`: hand swipes, pinches and holds to clients"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-05"
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

## Resolution (2026-11-05, PR #946)

Items 1 to 3 landed; item 4 and the real-pad check moved to
[`pointer-gestures-followups`](../protocols/pointer-gestures-followups.md).

1. **The global exists, opt-in.** `[gestures] client_gestures = true`
   advertises `zwp_pointer_gestures_v1` (version 3) once at startup. Off by
   default, since no real pad was available to check the toolkits against;
   a reload that disagrees is refused with `takes effect on restart`, like
   `[virtual_input] enabled`. The pinned fork's `PointerGesturesState` is
   generic, so there is no `delegate_pointer_gestures!` to invoke:
   `dispatch.rs`'s blanket `Dispatch` impls already cover the objects.
2. **Pinch and hold are forwarded.** The libinput arms call
   `gesture_pinch_*` and `gesture_hold_*` (`gesture/client.rs`), which
   remember whether a `begin` went out and drop an `update` or `end`
   otherwise; a `begin` over an open one closes it with a cancelled `end`
   first. Non-finite pinch values are dropped.
3. **A focus change mid-gesture needs nothing from scoot.** The ticket's
   premise, that the `end` goes to the new focus, does not hold at the pinned
   fork: each gesture object remembers the surface its `begin` was sent for,
   `PointerTarget::leave` sends the old focus a `cancelled` `end` before
   `wl_pointer.leave`, and the new focus's objects have nothing open so they
   hear no `update` or `end` (`src/wayland/pointer_gestures.rs`). Pinned for
   swipe, pinch and hold, to another client, to nowhere, and from nowhere
   onto a window, in `gesture/client/tests.rs`. The one case Smithay cannot
   cover is a surface destroyed mid-gesture: the client that destroyed it
   holds no state for it, and the object's next `begin` closes the stale one
   first.

No `wl_pointer.frame` follows a gesture event: the protocol does not group
them with pointer events, and Smithay's own anvil sends none.
