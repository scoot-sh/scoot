//! What the seat's pointer can be focused on (`SeatHandler::PointerFocus`):
//! a Wayland surface, or -- in an `xwayland` build -- an X11 window.
//!
//! # Why this is not simply `WlSurface`
//!
//! Pointer *events* do not need it: XWayland follows `wl_pointer.enter` and
//! motion on an X window's `wl_surface` by itself (unlike the keyboard --
//! see `keyboard_focus.rs`), so both variants deliver every
//! [`PointerTarget`] event to the surface, exactly as a plain `WlSurface`
//! focus did.
//!
//! Drops do. Smithay's `DnDGrab::new_pointer` pins a drag's target type to
//! `PointerFocus`, and a `WlSurface` target offers the drag through that
//! surface's `wl_data_device` -- which XWayland does not have (24.1.13 binds
//! none). An X window takes a drop only through the window manager's XDND
//! side, which Smithay implements as `DndFocus for X11Surface`
//! (`xwm/dnd.rs`): for a drag from Wayland it speaks XDND to the window on
//! the Wayland source's behalf. With a `WlSurface` focus that never ran,
//! and a drop from Wayland onto an X window did nothing.
//!
//! So the X variant carries the `X11Surface`, and a drag over it goes to
//! Smithay's X target. The surface beside it is what every other consumer
//! reads -- pointer delivery, constraints, the interaction-serial record --
//! through [`PointerFocus::surface`] or [`WaylandFocus`].
//!
//! Drags *from X* need that target too, for the other half of it: over an X
//! window it unmaps the window manager's full-screen XDND proxy, so the X
//! source finds the real window under the pointer and drops on it itself
//! (X to X, or within one X app), and it maps the proxy back when the drag
//! leaves, so a Wayland window is reachable again. That remap is flushed
//! only by the pinned scoot-sh/smithay fork's `6e6fe896`; without it an X
//! drag that had crossed an X window could no longer drop on a Wayland one
//! (`xwayland/tests/drop.rs`,
//! `an_x_drag_crossing_an_x_window_still_finds_the_proxy_over_wayland`).
//!
//! Without the `xwayland` feature this is a one-variant enum; every match on
//! it is exhaustive in both builds, so the default build's behaviour is the
//! `WlSurface` focus it always had.

use std::borrow::Cow;
use std::sync::Arc;

use smithay::backend::input::InputTime;
use smithay::desktop::Window;
use smithay::input::Seat;
use smithay::input::dnd::{DndFocus, OfferData, Source};
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
#[cfg(feature = "xwayland")]
use smithay::xwayland::X11Surface;
#[cfg(feature = "xwayland")]
use smithay::xwayland::xwm::XwmOfferData;

use super::State;
use super::keyboard_focus::KeyboardFocus;

/// The seat's pointer focus.
///
/// The X arm holds its window as a shared `Arc<X11Surface>`, made once per
/// window and reused by every hit test on it (see [`PointerFocus::on_window`]
/// and `State::x11_unmanaged`), rather than an `X11Surface` by value the
/// way `KeyboardFocus` does. Measured, not assumed: an `X11Surface` is 432
/// bytes (its connection's whole atom table) and about eight reference
/// counts, and a motion clones and drops the focus several times over --
/// the hit test, Smithay storing it and handing it back
/// (`current_focus`), the relative-motion event. By value that cost motion
/// over an X window ~0.6-0.7 us more in release (`x11_hot_path_cost`,
/// ~2.2 -> ~2.9 us), and every focus -- Wayland ones too -- was 496 bytes
/// to move. Shared, a clone is one reference count and the enum stays near
/// a `WlSurface`'s size.
#[derive(Debug, Clone)]
pub enum PointerFocus {
    /// Any Wayland surface: a toplevel's (or its subsurface's), a layer
    /// surface's, a lock surface's, a popup's.
    Surface(WlSurface),
    /// A managed or override-redirect X11 window, with the surface the hit
    /// test found on it -- its own `wl_surface`: XWayland makes one per X
    /// window and no subsurfaces, so that surface's origin is the window's,
    /// which is the origin Smithay's X drop target measures in.
    #[cfg(feature = "xwayland")]
    X11 {
        window: Arc<X11Surface>,
        surface: WlSurface,
    },
}

/// A managed X window's shared handle, kept in its Smithay `Window`'s user
/// data so every hit test on the window reuses one allocation. No cycle:
/// the `Window` holds this, and nothing this holds refers back to the
/// `Window` (an `X11Surface`'s own user data would -- see
/// `State::x11_unmanaged` for the override-redirect side).
#[cfg(feature = "xwayland")]
struct SharedX11(Arc<X11Surface>);

impl PointerFocus {
    /// The focus for `surface`, found by a hit test on `window`: the X
    /// variant when `window` is an X one. No lock and no allocation for a
    /// Wayland window (a match on Smithay's `Window` kind). An X window's
    /// costs a user-data lookup and one reference count; its shared handle
    /// is allocated the first time the pointer finds the window, once per
    /// window, never per event.
    pub fn on_window(window: &Window, surface: WlSurface) -> Self {
        #[cfg(feature = "xwayland")]
        if let Some(x11) = window.x11_surface() {
            let shared = window
                .user_data()
                .get_or_insert(|| SharedX11(Arc::new(x11.clone())));
            return Self::X11 {
                window: Arc::clone(&shared.0),
                surface,
            };
        }
        #[cfg(not(feature = "xwayland"))]
        let _ = window;
        Self::Surface(surface)
    }

    /// The surface pointer events are delivered to.
    pub fn surface(&self) -> &WlSurface {
        match self {
            Self::Surface(surface) => surface,
            #[cfg(feature = "xwayland")]
            Self::X11 { surface, .. } => surface,
        }
    }

    /// The same surface, owned: for the paths that address the surface
    /// itself rather than the focus (a tablet tool's `ToolFocus`).
    pub fn into_surface(self) -> WlSurface {
        match self {
            Self::Surface(surface) => surface,
            #[cfg(feature = "xwayland")]
            Self::X11 { surface, .. } => surface,
        }
    }
}

/// The same focus: the same variant on the same surface (and, for the X
/// variant, the same X window of the same window manager).
///
/// Written out rather than derived, because the derive would compare the X
/// arm with `X11Surface`'s own `PartialEq`, which takes both windows' state
/// locks to fold liveness in -- and Smithay compares the new focus with the
/// old on *every* pointer motion (`PointerInternal::motion`), so that would
/// be two locks per motion over an X window. Liveness is not identity: a
/// dead surface is still the focus it was until the hit test stops finding
/// it, which is how the `WlSurface` focus has always compared, and liveness
/// is asked separately where it matters ([`IsAlive`]: Smithay's grab and
/// drag paths). The window id and window manager are plain fields; the
/// surface decides almost every comparison first.
impl PartialEq for PointerFocus {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Surface(a), Self::Surface(b)) => a == b,
            #[cfg(feature = "xwayland")]
            (
                Self::X11 {
                    window: a_window,
                    surface: a,
                },
                Self::X11 {
                    window: b_window,
                    surface: b,
                },
            ) => {
                a == b
                    && a_window.window_id() == b_window.window_id()
                    && a_window.xwm_id() == b_window.xwm_id()
            }
            #[cfg(feature = "xwayland")]
            _ => false,
        }
    }
}

impl From<WlSurface> for PointerFocus {
    fn from(surface: WlSurface) -> Self {
        Self::Surface(surface)
    }
}

/// What Smithay's popup grab starts its pointer grab on: the keyboard focus
/// of the grab's root, converted once as the grab is made. Carried over
/// variant for variant, so an X window stays the X focus the hit test would
/// name it; that arm allocates its handle, which is fine for a once-per-grab
/// conversion that is not reached anyway -- an `xdg_popup` grab's root is
/// an xdg toplevel, never an X window.
impl From<KeyboardFocus> for PointerFocus {
    fn from(focus: KeyboardFocus) -> Self {
        match focus {
            KeyboardFocus::Surface(surface) => Self::Surface(surface),
            #[cfg(feature = "xwayland")]
            KeyboardFocus::X11 { window, surface } => Self::X11 {
                window: Arc::new(window),
                surface,
            },
        }
    }
}

impl IsAlive for PointerFocus {
    fn alive(&self) -> bool {
        match self {
            Self::Surface(surface) => surface.alive(),
            #[cfg(feature = "xwayland")]
            Self::X11 { window, surface } => window.alive() && surface.alive(),
        }
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

/// Every event forwarded, unchanged, to the surface's own impl -- for an X
/// window too, which is all Smithay's `X11Surface` pointer target does (it
/// forwards to the window's current `wl_surface`, under the window's state
/// lock, per event). `replace` keeps Smithay's default (leave the old focus,
/// reset the cursor, enter this one), which is what the `WlSurface` focus
/// got as well: Smithay's `WlSurface` impl does not override it.
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

/// What a drag offered the focus it entered: the `wl_data_offer` state of a
/// Wayland target, or the XDND offer state of an X one.
///
/// `DnDGrab` keeps the offer beside the focus it came from, drops it on
/// every focus change, and asks the new focus's `enter` for the next one --
/// so an offer is always the kind its focus's `enter` made. The methods
/// below still match focus and offer together rather than trust that: an X
/// focus reaches Smithay's X target with an X offer or none, and anything
/// else goes the surface's way with a surface offer or none -- harmless (no
/// offer is what a target that declined `enter` has anyway), where a panic
/// would take the session down.
#[derive(Debug)]
pub enum PointerOffer<S: Source> {
    Surface(WlOfferData<S>),
    #[cfg(feature = "xwayland")]
    X11(XwmOfferData<S>),
}

impl<S: Source> OfferData for PointerOffer<S> {
    fn disable(&self) {
        match self {
            Self::Surface(offer) => offer.disable(),
            #[cfg(feature = "xwayland")]
            Self::X11(offer) => offer.disable(),
        }
    }

    fn drop(&self) {
        match self {
            Self::Surface(offer) => OfferData::drop(offer),
            #[cfg(feature = "xwayland")]
            Self::X11(offer) => OfferData::drop(offer),
        }
    }

    fn validated(&self) -> bool {
        match self {
            Self::Surface(offer) => offer.validated(),
            #[cfg(feature = "xwayland")]
            Self::X11(offer) => offer.validated(),
        }
    }
}

impl<S: Source> PointerOffer<S> {
    fn as_surface(&mut self) -> Option<&mut WlOfferData<S>> {
        match self {
            Self::Surface(offer) => Some(offer),
            #[cfg(feature = "xwayland")]
            Self::X11(_) => None,
        }
    }

    #[cfg(feature = "xwayland")]
    fn as_x11(&mut self) -> Option<&mut XwmOfferData<S>> {
        match self {
            Self::X11(offer) => Some(offer),
            Self::Surface(_) => None,
        }
    }
}

/// A drag's target: the surface's `wl_data_device`, or -- for an X window
/// -- Smithay's XDND target (see the module doc), which handles both kinds
/// of drag: for one from Wayland it speaks XDND to the window and answers
/// an X offer; for one from X it answers no offer and moves the window
/// manager's proxy out of the way (and back on `leave`), and its `drop`
/// ends the window manager's side of the X drag.
///
/// So an X focus goes to the X target with an X offer *or none*, and only
/// an X focus reaches it. A surface offer on an X focus cannot happen
/// (`DnDGrab` keeps each offer beside the focus whose `enter` made it, and
/// an X focus's `enter` makes only X offers); it goes the surface's way,
/// like every mismatch, rather than panicking.
impl DndFocus<State> for PointerFocus {
    type OfferData<S>
        = PointerOffer<S>
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
        match self {
            #[cfg(feature = "xwayland")]
            Self::X11 { window, .. } => {
                DndFocus::enter(&**window, data, dh, source, seat, location, serial)
                    .map(PointerOffer::X11)
            }
            focus => DndFocus::enter(focus.surface(), data, dh, source, seat, location, serial)
                .map(PointerOffer::Surface),
        }
    }

    fn motion<S: Source>(
        &self,
        data: &mut State,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<State>,
        location: Point<f64, Logical>,
        time: InputTime,
    ) {
        match (self, offer) {
            #[cfg(feature = "xwayland")]
            (Self::X11 { window, .. }, offer @ (None | Some(PointerOffer::X11(_)))) => {
                let offer = offer.and_then(PointerOffer::as_x11);
                DndFocus::motion(&**window, data, offer, seat, location, time);
            }
            (focus, offer) => {
                let offer = offer.and_then(PointerOffer::as_surface);
                DndFocus::motion(focus.surface(), data, offer, seat, location, time);
            }
        }
    }

    fn leave<S: Source>(
        &self,
        data: &mut State,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<State>,
    ) {
        match (self, offer) {
            #[cfg(feature = "xwayland")]
            (Self::X11 { window, .. }, offer @ (None | Some(PointerOffer::X11(_)))) => {
                let offer = offer.and_then(PointerOffer::as_x11);
                DndFocus::leave(&**window, data, offer, seat);
            }
            (focus, offer) => {
                let offer = offer.and_then(PointerOffer::as_surface);
                DndFocus::leave(focus.surface(), data, offer, seat);
            }
        }
    }

    fn drop<S: Source>(
        &self,
        data: &mut State,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<State>,
    ) {
        match (self, offer) {
            #[cfg(feature = "xwayland")]
            (Self::X11 { window, .. }, offer @ (None | Some(PointerOffer::X11(_)))) => {
                let offer = offer.and_then(PointerOffer::as_x11);
                DndFocus::drop(&**window, data, offer, seat);
            }
            (focus, offer) => {
                let offer = offer.and_then(PointerOffer::as_surface);
                DndFocus::drop(focus.surface(), data, offer, seat);
            }
        }
    }
}
