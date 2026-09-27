//! What the seat's pointer can be focused on (`SeatHandler::PointerFocus`).
//!
//! A newtype-shaped enum over the `WlSurface` it always was: every pointer
//! event is delivered to that surface exactly as before, through Smithay's
//! own `WlSurface` impls. It exists so a drag-and-drop grab -- whose target
//! type Smithay pins to `PointerFocus` (`DnDGrab::new_pointer`) -- can be
//! given a second kind of target without touching any other pointer path.
//!
//! One variant for now, and every match on it is exhaustive, so this is the
//! `WlSurface` focus it replaced, behaviour for behaviour.

use std::borrow::Cow;
use std::sync::Arc;

use smithay::backend::input::InputTime;
use smithay::input::Seat;
use smithay::input::dnd::{DndFocus, Source};
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
    GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
    GestureSwipeUpdateEvent, MotionEvent, PointerTarget, RelativeMotionEvent,
};
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{DisplayHandle, Resource};
use smithay::utils::{IsAlive, Logical, Point, Serial};
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::selection::data_device::WlOfferData;

use super::State;
use super::keyboard_focus::KeyboardFocus;

/// The seat's pointer focus.
#[derive(Debug, Clone, PartialEq)]
pub enum PointerFocus {
    /// Any surface the pointer can be over: a window's, a layer surface's,
    /// a lock surface's, a popup's, an X window's.
    Surface(WlSurface),
}

impl PointerFocus {
    /// The surface pointer events are delivered to.
    pub fn surface(&self) -> &WlSurface {
        match self {
            Self::Surface(surface) => surface,
        }
    }

    /// The same surface, owned: for the paths that address the surface
    /// itself rather than the focus (a tablet tool's `ToolFocus`).
    pub fn into_surface(self) -> WlSurface {
        match self {
            Self::Surface(surface) => surface,
        }
    }
}

impl From<WlSurface> for PointerFocus {
    fn from(surface: WlSurface) -> Self {
        Self::Surface(surface)
    }
}

/// Smithay's popup grab hands the pointer to whatever the keyboard focus of
/// the grab's root was: always a surface the pointer can be delivered to.
impl From<KeyboardFocus> for PointerFocus {
    fn from(focus: KeyboardFocus) -> Self {
        Self::Surface(focus.into())
    }
}

impl IsAlive for PointerFocus {
    fn alive(&self) -> bool {
        self.surface().alive()
    }
}

impl WaylandFocus for PointerFocus {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        Some(Cow::Borrowed(self.surface()))
    }

    fn same_client_as(&self, object_id: &ObjectId) -> bool {
        self.surface().id().same_client_as(object_id)
    }
}

/// Every event forwarded, unchanged, to the surface's own impl. `replace`
/// keeps Smithay's default (leave the old focus, reset the cursor, enter
/// this one), which is what the `WlSurface` focus got too: Smithay's
/// `WlSurface` impl does not override it.
impl PointerTarget<State> for PointerFocus {
    fn enter(&self, seat: &Seat<State>, data: &mut State, event: &MotionEvent) {
        PointerTarget::enter(self.surface(), seat, data, event);
    }

    fn motion(&self, seat: &Seat<State>, data: &mut State, event: &MotionEvent) {
        PointerTarget::motion(self.surface(), seat, data, event);
    }

    fn relative_motion(&self, seat: &Seat<State>, data: &mut State, event: &RelativeMotionEvent) {
        PointerTarget::relative_motion(self.surface(), seat, data, event);
    }

    fn button(&self, seat: &Seat<State>, data: &mut State, event: &ButtonEvent) {
        PointerTarget::button(self.surface(), seat, data, event);
    }

    fn axis(&self, seat: &Seat<State>, data: &mut State, frame: AxisFrame) {
        PointerTarget::axis(self.surface(), seat, data, frame);
    }

    fn frame(&self, seat: &Seat<State>, data: &mut State) {
        PointerTarget::frame(self.surface(), seat, data);
    }

    fn gesture_swipe_begin(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        event: &GestureSwipeBeginEvent,
    ) {
        PointerTarget::gesture_swipe_begin(self.surface(), seat, data, event);
    }

    fn gesture_swipe_update(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        event: &GestureSwipeUpdateEvent,
    ) {
        PointerTarget::gesture_swipe_update(self.surface(), seat, data, event);
    }

    fn gesture_swipe_end(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        event: &GestureSwipeEndEvent,
    ) {
        PointerTarget::gesture_swipe_end(self.surface(), seat, data, event);
    }

    fn gesture_pinch_begin(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        event: &GesturePinchBeginEvent,
    ) {
        PointerTarget::gesture_pinch_begin(self.surface(), seat, data, event);
    }

    fn gesture_pinch_update(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        event: &GesturePinchUpdateEvent,
    ) {
        PointerTarget::gesture_pinch_update(self.surface(), seat, data, event);
    }

    fn gesture_pinch_end(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        event: &GesturePinchEndEvent,
    ) {
        PointerTarget::gesture_pinch_end(self.surface(), seat, data, event);
    }

    fn gesture_hold_begin(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        event: &GestureHoldBeginEvent,
    ) {
        PointerTarget::gesture_hold_begin(self.surface(), seat, data, event);
    }

    fn gesture_hold_end(&self, seat: &Seat<State>, data: &mut State, event: &GestureHoldEndEvent) {
        PointerTarget::gesture_hold_end(self.surface(), seat, data, event);
    }

    fn leave(&self, seat: &Seat<State>, data: &mut State, serial: Serial, time: InputTime) {
        PointerTarget::leave(self.surface(), seat, data, serial, time);
    }
}

/// A drag's target: the surface's `wl_data_device`, as before.
impl DndFocus<State> for PointerFocus {
    type OfferData<S>
        = WlOfferData<S>
    where
        S: Source;

    fn enter<S: Source>(
        &self,
        data: &mut State,
        dh: &DisplayHandle,
        source: Arc<S>,
        seat: &Seat<State>,
        location: Point<f64, Logical>,
        serial: &Serial,
    ) -> Option<Self::OfferData<S>> {
        DndFocus::enter(self.surface(), data, dh, source, seat, location, serial)
    }

    fn motion<S: Source>(
        &self,
        data: &mut State,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<State>,
        location: Point<f64, Logical>,
        time: InputTime,
    ) {
        DndFocus::motion(self.surface(), data, offer, seat, location, time);
    }

    fn leave<S: Source>(
        &self,
        data: &mut State,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<State>,
    ) {
        DndFocus::leave(self.surface(), data, offer, seat);
    }

    fn drop<S: Source>(
        &self,
        data: &mut State,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<State>,
    ) {
        DndFocus::drop(self.surface(), data, offer, seat);
    }
}
