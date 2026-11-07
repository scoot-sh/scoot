//! The host's `zwp_pointer_gestures_v1`, routed into the same entry points
//! the libinput arms of `--tty` call (`gesture.rs` for a swipe,
//! `gesture/client.rs` for a pinch and a hold), so `[gestures]` and the
//! client forwarding work in a nested session.
//!
//! # What is bound, and when
//!
//! The manager is bound at startup if the host offers it, at any version
//! from 1 to 3. A host without it is the quiet case: no object is made, no
//! gesture arrives, and the session is exactly what it was before this
//! module existed (a `debug` line says why). The gesture objects belong to
//! the host `wl_pointer`, so they are made when the host seat gains a
//! pointer and destroyed when it loses one (see [`State::host_pointer_lost`]).
//!
//! What is asked of the host follows what scoot would do with it
//! ([`Plan`]):
//!
//! - **swipe**, always: scoot's own `[gestures]` swipe works whether or not
//!   the client global is on, as on `--tty`, where the libinput swipe arms
//!   call `gesture_swipe_*` unconditionally.
//! - **pinch**, only with `[gestures] client_gestures`: scoot binds nothing
//!   to a pinch, and with no global of its own to send it through
//!   `gesture_pinch_*` drops it. Not asking saves the host the traffic.
//! - **hold**, only with `client_gestures` and a host at version 3 or more
//!   (`get_hold_gesture` is a version-3 request; sending it to an older host
//!   is a protocol error that kills the connection).
//!
//! `client_gestures` is fixed for the life of the session (a reload that
//! disagrees is refused), so the plan is made once per pointer.
//!
//! # Begin decides, end matches
//!
//! The entry points keep the rule per gesture (`gesture.rs`, `client.rs`).
//! [`Open`] keeps it one layer earlier, for what the host sends: an `update`
//! or `end` with no `begin` before it never reaches an entry point, so a
//! host that restarts a sequence, or an event in flight from an object being
//! destroyed, cannot turn into an `update` for a gesture nobody began. A
//! `begin` over an open one is passed through unclosed, as the libinput arms
//! of `--tty` pass it; what closes the old gesture `cancelled` first is
//! `gesture_swipe_begin` for a swipe and Smithay's gesture object for a
//! pinch or hold (`gesture/client.rs`).
//!
//! A gesture the host never ends is cancelled here, not left open: when the
//! host pointer goes away, and when it leaves scoot's window (a conformant
//! host sends the cancelled `end` itself first, after which this has nothing
//! to cancel). Events carry the host's own millisecond clock, as libinput's
//! carry its own.
//!
//! The host delivers `f64`s decoded from 24.8 fixed point, so a non-finite
//! value cannot arrive; the pinch entry point drops one anyway.
//!
//! # Cost
//!
//! No allocation per event: [`HostGesture`] is a `Copy` value, [`Open`] three
//! flags, and the objects are made once per host pointer.

use wayland_client::globals::GlobalList;
use wayland_client::protocol::wl_pointer::WlPointer as HostPointer;
use wayland_client::{Proxy, QueueHandle};
use wayland_protocols::wp::pointer_gestures::zv1::client::zwp_pointer_gesture_hold_v1::ZwpPointerGestureHoldV1;
use wayland_protocols::wp::pointer_gestures::zv1::client::zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1;
use wayland_protocols::wp::pointer_gestures::zv1::client::zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1;
use wayland_protocols::wp::pointer_gestures::zv1::client::zwp_pointer_gestures_v1::ZwpPointerGesturesV1;

use super::super::State;

#[cfg(test)]
mod tests;

/// The first version with `get_hold_gesture`.
const HOLD_VERSION: u32 = 3;

/// The newest version scoot binds. Tests lower it to look like an older
/// host, since the only host a test can run is Smithay's, fixed at 3.
#[cfg(not(test))]
fn bind_cap() -> u32 {
    3
}

#[cfg(test)]
thread_local! {
    pub(super) static BIND_CAP: std::cell::Cell<u32> = const { std::cell::Cell::new(3) };
}

#[cfg(test)]
fn bind_cap() -> u32 {
    BIND_CAP.with(std::cell::Cell::get)
}

/// One host gesture event, reduced to what the entry points take.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::compositor) enum HostGesture {
    SwipeBegin {
        fingers: u32,
        time: u32,
    },
    SwipeUpdate {
        dx: f64,
        dy: f64,
        time: u32,
    },
    SwipeEnd {
        cancelled: bool,
        time: u32,
    },
    PinchBegin {
        fingers: u32,
        time: u32,
    },
    PinchUpdate {
        dx: f64,
        dy: f64,
        scale: f64,
        rotation: f64,
        time: u32,
    },
    PinchEnd {
        cancelled: bool,
        time: u32,
    },
    HoldBegin {
        fingers: u32,
        time: u32,
    },
    HoldEnd {
        cancelled: bool,
        time: u32,
    },
}

impl HostGesture {
    fn time(&self) -> u32 {
        use HostGesture::*;
        match *self {
            SwipeBegin { time, .. }
            | SwipeUpdate { time, .. }
            | SwipeEnd { time, .. }
            | PinchBegin { time, .. }
            | PinchUpdate { time, .. }
            | PinchEnd { time, .. }
            | HoldBegin { time, .. }
            | HoldEnd { time, .. } => time,
        }
    }
}

/// Which gestures the host has begun and not ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::compositor) struct Open {
    swipe: bool,
    pinch: bool,
    hold: bool,
}

impl Open {
    /// Whether `event` follows a `begin` and so goes on to an entry point,
    /// recording what it changes. An `end` closes its gesture even when the
    /// entry point later drops it.
    pub(super) fn admit(&mut self, event: &HostGesture) -> bool {
        use HostGesture::*;
        match event {
            SwipeBegin { .. } => self.swipe = true,
            PinchBegin { .. } => self.pinch = true,
            HoldBegin { .. } => self.hold = true,
            SwipeUpdate { .. } => return self.swipe,
            PinchUpdate { .. } => return self.pinch,
            SwipeEnd { .. } => return std::mem::take(&mut self.swipe),
            PinchEnd { .. } => return std::mem::take(&mut self.pinch),
            HoldEnd { .. } => return std::mem::take(&mut self.hold),
        }
        true
    }

    /// The cancelled `end`s that close everything still open.
    fn cancels(self, time: u32) -> impl Iterator<Item = HostGesture> {
        [
            self.swipe.then_some(HostGesture::SwipeEnd {
                cancelled: true,
                time,
            }),
            self.pinch.then_some(HostGesture::PinchEnd {
                cancelled: true,
                time,
            }),
            self.hold.then_some(HostGesture::HoldEnd {
                cancelled: true,
                time,
            }),
        ]
        .into_iter()
        .flatten()
    }
}

/// Which gesture objects to ask a host of a given version for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Plan {
    pub(super) swipe: bool,
    pub(super) pinch: bool,
    pub(super) hold: bool,
}

impl Plan {
    pub(super) fn new(host_version: u32, client_gestures: bool) -> Self {
        Self {
            swipe: true,
            pinch: client_gestures,
            hold: client_gestures && host_version >= HOLD_VERSION,
        }
    }
}

/// The gesture objects of one host pointer.
struct Objects {
    swipe: Option<ZwpPointerGestureSwipeV1>,
    pinch: Option<ZwpPointerGesturePinchV1>,
    hold: Option<ZwpPointerGestureHoldV1>,
}

/// The host's pointer-gestures manager and the objects made from it.
#[derive(Default)]
pub(super) struct HostGestures {
    /// `None`: the host does not offer the global.
    manager: Option<ZwpPointerGesturesV1>,
    objects: Option<Objects>,
    open: Open,
    /// The host clock of the last event passed on, which a cancel
    /// synthesized here is stamped with, so a gesture keeps one timeline.
    last_time: u32,
}

impl HostGestures {
    /// Binds the manager if the host has one, at the newest version scoot
    /// knows (3) the host offers.
    pub(super) fn bind(globals: &GlobalList, qh: &QueueHandle<State>) -> Self {
        Self::bind_up_to(globals, qh, bind_cap())
    }

    fn bind_up_to(globals: &GlobalList, qh: &QueueHandle<State>, max: u32) -> Self {
        let manager = match globals.bind::<ZwpPointerGesturesV1, _, _>(qh, 1..=max, ()) {
            Ok(manager) => {
                tracing::debug!(
                    version = manager.version(),
                    "nested: the host offers zwp_pointer_gestures_v1"
                );
                Some(manager)
            }
            Err(error) => {
                tracing::debug!(%error, "nested: no host gestures; touchpad gestures do not reach this session");
                None
            }
        };
        Self {
            manager,
            ..Self::default()
        }
    }

    /// Makes the gesture objects for a new host `pointer`, as `Plan` says.
    /// Nothing without the global, or with objects already made.
    pub(super) fn attach(
        &mut self,
        pointer: &HostPointer,
        qh: &QueueHandle<State>,
        client_gestures: bool,
    ) {
        let Some(manager) = &self.manager else {
            return;
        };
        if self.objects.is_some() {
            return;
        }
        let plan = Plan::new(manager.version(), client_gestures);
        self.objects = Some(Objects {
            swipe: plan
                .swipe
                .then(|| manager.get_swipe_gesture(pointer, qh, ())),
            pinch: plan
                .pinch
                .then(|| manager.get_pinch_gesture(pointer, qh, ())),
            hold: plan.hold.then(|| manager.get_hold_gesture(pointer, qh, ())),
        });
    }

    /// Destroys the objects (their pointer is going) and takes what was
    /// still open, for the caller to cancel.
    pub(super) fn detach(&mut self) -> Open {
        if let Some(objects) = self.objects.take() {
            if let Some(swipe) = objects.swipe {
                swipe.destroy();
            }
            if let Some(pinch) = objects.pinch {
                pinch.destroy();
            }
            if let Some(hold) = objects.hold {
                hold.destroy();
            }
        }
        std::mem::take(&mut self.open)
    }

    /// Takes what was still open, leaving the objects.
    pub(super) fn take_open(&mut self) -> Open {
        std::mem::take(&mut self.open)
    }

    /// Notes an admitted event's host timestamp.
    fn note_time(&mut self, event: &HostGesture) {
        self.last_time = event.time();
    }

    #[cfg(test)]
    fn bound(&self) -> Option<u32> {
        self.manager.as_ref().map(Proxy::version)
    }

    #[cfg(test)]
    fn plan_made(&self) -> Option<Plan> {
        self.objects.as_ref().map(|o| Plan {
            swipe: o.swipe.is_some(),
            pinch: o.pinch.is_some(),
            hold: o.hold.is_some(),
        })
    }
}

impl State {
    /// One event from a host gesture object. Dropped unless it follows a
    /// `begin` (see [`Open`]); otherwise it goes to the entry point `--tty`
    /// would call.
    pub(in crate::compositor) fn host_gesture(&mut self, event: HostGesture) {
        let Some(host) = &mut self.host else {
            return;
        };
        if !host.gestures.open.admit(&event) {
            return;
        }
        host.gestures.note_time(&event);
        self.apply_host_gesture(event);
    }

    fn apply_host_gesture(&mut self, event: HostGesture) {
        use HostGesture::*;
        match event {
            SwipeBegin { fingers, time } => self.gesture_swipe_begin(fingers, time),
            SwipeUpdate { dx, dy, time } => self.gesture_swipe_update(dx, dy, time),
            SwipeEnd { cancelled, time } => self.gesture_swipe_end(cancelled, time),
            PinchBegin { fingers, time } => self.gesture_pinch_begin(fingers, time),
            PinchUpdate {
                dx,
                dy,
                scale,
                rotation,
                time,
            } => self.gesture_pinch_update((dx, dy), scale, rotation, time),
            PinchEnd { cancelled, time } => self.gesture_pinch_end(cancelled, time),
            HoldBegin { fingers, time } => self.gesture_hold_begin(fingers, time),
            HoldEnd { cancelled, time } => self.gesture_hold_end(cancelled, time),
        }
    }

    /// Closes, `cancelled`, every gesture the host began and did not end,
    /// on the host's own clock (the last event's).
    fn cancel_open_host_gestures(&mut self, open: Open, time: u32) {
        for event in open.cancels(time) {
            self.apply_host_gesture(event);
        }
    }

    /// The host pointer left scoot's window: whatever it began and did not
    /// end is over.
    pub(in crate::compositor) fn host_pointer_left(&mut self) {
        let Some(host) = &mut self.host else {
            return;
        };
        let open = host.gestures.take_open();
        let time = host.gestures.last_time;
        self.cancel_open_host_gestures(open, time);
    }

    /// The host seat no longer has a pointer: its gesture objects go first
    /// (children before parent), then the pointer is released, and what it
    /// had in flight is dropped: a half-buffered scroll, and the gestures
    /// still open end cancelled.
    pub(in crate::compositor) fn host_pointer_lost(&mut self) {
        let Some(host) = &mut self.host else {
            return;
        };
        let open = host.gestures.detach();
        let time = host.gestures.last_time;
        let _ = host.take_pointer();
        let _ = host.take_pending_axis();
        self.cancel_open_host_gestures(open, time);
    }
}
