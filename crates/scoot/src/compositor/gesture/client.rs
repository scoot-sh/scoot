//! What a client hears of a gesture: the `zwp_pointer_gestures_v1` global and
//! the pinch and hold halves of the forwarding (the swipe half lives in
//! `gesture.rs`, because scoot may claim a swipe for itself).
//!
//! # The global is opt-in
//!
//! `[gestures] client_gestures = true` advertises the global, once at
//! startup ([`init_client_gestures`]). It is off by default because a bound
//! global changes what toolkits do with a touchpad (GTK4, Firefox and
//! Chromium switch to the gesture events for pinch zoom and swipe
//! navigation), and that has not been checked against a real pad. A reload
//! that disagrees with the running session is refused (see `reload.rs`).
//! Off, there is no global, no client holds a gesture object, and every
//! forwarded event below reaches nobody.
//!
//! The pinned Smithay (the `scoot-sh` fork, `src/wayland/pointer_gestures.rs`)
//! is generic over the state, so there is no `delegate_pointer_gestures!`
//! here: `dispatch.rs`'s blanket `Dispatch` impls already route the global
//! and its three object kinds through Smithay's `Dispatch2` impls.
//!
//! # Pinch and hold are never scoot's
//!
//! Scoot binds nothing to them, so both are forwarded whole to the pointer
//! focus. There is no route to decide, unlike a swipe (`gesture.rs`), and no
//! state of scoot's own to keep: the rule `gesture.rs` states -- no client
//! gets an `update` or `end` for a `begin` it did not get -- is kept per
//! gesture object by Smithay, which remembers the surface each `begin` was
//! sent for (`in_progress_on`, `src/wayland/pointer_gestures.rs`). An `update`
//! or `end` for a `begin` that object never sent is dropped, and a second
//! `begin` first ends the open one `cancelled`. A flag of scoot's own beside
//! that would never change what a client receives (no sequence of begins,
//! focus changes and ends was found where it does), so there is none; the
//! tests pin what a client sees, which is what a Smithay repin must keep.
//!
//! Scoot never abandons a pinch or hold, so it never has to cancel one.
//! Non-finite values are dropped before they reach the wire.
//!
//! With no global the events are dropped before anything else, as they were
//! before this module existed: no idle-notify activity, no cursor reshow.
//!
//! # A focus change in the middle of a gesture
//!
//! Smithay already does what a gesture needs here, for all three kinds, so
//! scoot adds nothing on top (verified against the pinned source, and pinned
//! by `gesture/client/tests.rs`):
//!
//! - Each gesture object remembers the surface its `begin` was sent for.
//!   When the pointer leaves that surface, `PointerTarget::leave` sends the
//!   old focus's client a `cancelled` `end` (once) before `wl_pointer.leave`.
//! - The new focus's client has no gesture open, so it gets no `update`
//!   and no `end` for a `begin` it never saw, whether it is another client or
//!   another surface of the same one.
//!
//! The same holds when the new focus is another surface of the same client:
//! `leave` takes the open gesture off the object before the new surface is
//! entered. What it cannot do is end a gesture on a surface that is already
//! destroyed; the client that destroyed it holds no state for it, and the
//! object's next `begin` closes the stale one first.
//!
//! No `wl_pointer.frame` follows a gesture event: the protocol does not
//! group them with pointer events, and a frame at update rate would only
//! wake the focused client for nothing.
//!
//! # Cost
//!
//! No allocation per event: the events are stack values and Smithay clones
//! reference-counted handles. With no global bound, the lists it walks are
//! empty.

use smithay::backend::input::InputTime;
use smithay::input::pointer::{
    GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent,
    GesturePinchUpdateEvent,
};
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::pointer_gestures::PointerGesturesState;

use crate::compositor::State;

#[cfg(test)]
mod tests;

/// Advertises `zwp_pointer_gestures_v1` when `enabled`
/// (`[gestures] client_gestures`). Called once from `compositor::run`.
pub(in crate::compositor) fn init_client_gestures(state: &mut State, enabled: bool) {
    if enabled {
        state.client_gestures = Some(PointerGesturesState::new::<State>(&state.display_handle));
    }
}

impl State {
    /// Whether a gesture has anyone to go to. Off, the libinput arms drop the
    /// event here, so the default session does exactly what it did when
    /// these arms were `_ => {}`.
    fn gestures_offered(&self) -> bool {
        self.client_gestures.is_some()
    }

    /// A pinch began with `fingers` on the pad.
    pub(in crate::compositor) fn gesture_pinch_begin(&mut self, fingers: u32, time: u32) {
        if !self.gestures_offered() {
            return;
        }
        self.announce_activity();
        self.note_pointer_activity();
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let event = GesturePinchBeginEvent {
            serial: SERIAL_COUNTER.next_serial(),
            time: InputTime::from_millis(time),
            fingers,
        };
        pointer.gesture_pinch_begin(self, &event);
    }

    /// The pinch moved: `(dx, dy)` of its centre since the last event,
    /// `scale` against the `begin` and `rotation` in degrees clockwise since
    /// the last event.
    pub(in crate::compositor) fn gesture_pinch_update(
        &mut self,
        (dx, dy): (f64, f64),
        scale: f64,
        rotation: f64,
        time: u32,
    ) {
        if !self.gestures_offered() {
            return;
        }
        self.announce_activity();
        self.note_pointer_activity();
        // A value no pad produces: nothing to send.
        if !(dx.is_finite() && dy.is_finite() && scale.is_finite() && rotation.is_finite()) {
            return;
        }
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let event = GesturePinchUpdateEvent {
            time: InputTime::from_millis(time),
            delta: (dx, dy).into(),
            scale,
            rotation,
        };
        pointer.gesture_pinch_update(self, &event);
    }

    /// The fingers lifted (`cancelled` is false) or the pad gave up.
    pub(in crate::compositor) fn gesture_pinch_end(&mut self, cancelled: bool, time: u32) {
        if !self.gestures_offered() {
            return;
        }
        self.announce_activity();
        self.note_pointer_activity();
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let event = GesturePinchEndEvent {
            serial: SERIAL_COUNTER.next_serial(),
            time: InputTime::from_millis(time),
            cancelled,
        };
        pointer.gesture_pinch_end(self, &event);
    }

    /// `fingers` came to rest on the pad.
    pub(in crate::compositor) fn gesture_hold_begin(&mut self, fingers: u32, time: u32) {
        if !self.gestures_offered() {
            return;
        }
        self.announce_activity();
        self.note_pointer_activity();
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let event = GestureHoldBeginEvent {
            serial: SERIAL_COUNTER.next_serial(),
            time: InputTime::from_millis(time),
            fingers,
        };
        pointer.gesture_hold_begin(self, &event);
    }

    /// The fingers lifted (`cancelled` is false) or moved on to something
    /// else, which libinput reports as a cancelled hold.
    pub(in crate::compositor) fn gesture_hold_end(&mut self, cancelled: bool, time: u32) {
        if !self.gestures_offered() {
            return;
        }
        self.announce_activity();
        self.note_pointer_activity();
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let event = GestureHoldEndEvent {
            serial: SERIAL_COUNTER.next_serial(),
            time: InputTime::from_millis(time),
            cancelled,
        };
        pointer.gesture_hold_end(self, &event);
    }
}
