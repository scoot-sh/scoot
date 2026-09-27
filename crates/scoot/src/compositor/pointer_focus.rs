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
//! So the X variant carries the `X11Surface`, and a drag from Wayland over
//! it goes to Smithay's X target. The surface beside it is what every other
//! consumer reads -- pointer delivery, constraints, the interaction-serial
//! record -- through [`PointerFocus::surface`] or [`WaylandFocus`].
//!
//! Drags *from X* onto X windows (X to X, or within one X app) need the
//! other half of Smithay's X target -- unmapping the window manager's
//! full-screen proxy over X windows, so the X source finds the real window
//! and drops on it itself -- and that half has a bug at the pinned rev that
//! would break X-to-Wayland drops, which work today; they stay on the
//! surface's path until a fork commit fixes it (see the `DndFocus` impl).
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
/// The X arm is large (an `X11Surface` carries its connection's whole atom
/// table by value -- 432 bytes against a `WlSurface`'s 64 at the pinned
/// rev), and deliberately not boxed, for the reason `KeyboardFocus` gives:
/// the focus is built by every hit test and cloned into the seat on every
/// motion, and a box would make each of those a heap allocation. Inline,
/// the clone is reference-count bumps. What the size costs a motion over a
/// *Wayland* window -- the moves of a larger value -- is measured in the
/// `pointer_motion` bench (see the PR that introduced this arm).
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
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
        window: X11Surface,
        surface: WlSurface,
    },
}

impl PointerFocus {
    /// The focus for `surface`, found by a hit test on `window`: the X
    /// variant when `window` is an X one. No lock and no allocation for a
    /// Wayland window (a match on Smithay's `Window` kind); an X window's
    /// costs the `X11Surface`'s reference-count bumps.
    pub fn on_window(window: &Window, surface: WlSurface) -> Self {
        #[cfg(feature = "xwayland")]
        if let Some(x11) = window.x11_surface() {
            return Self::X11 {
                window: x11.clone(),
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

/// What Smithay's popup grab hands the pointer when it restores the grab's
/// root: the keyboard focus the root had, carried over variant for variant,
/// so an X window stays the X focus the hit test would name it.
impl From<KeyboardFocus> for PointerFocus {
    fn from(focus: KeyboardFocus) -> Self {
        match focus {
            KeyboardFocus::Surface(surface) => Self::Surface(surface),
            #[cfg(feature = "xwayland")]
            KeyboardFocus::X11 { window, surface } => Self::X11 { window, surface },
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
/// below still match focus and offer together rather than trust that: only
/// an X offer on an X focus reaches Smithay's X target, and anything else
/// goes the surface's way with a surface offer or none -- harmless (no
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
}

/// Whether a drag over `window` comes from X -- from the X server `window`
/// belongs to -- by Smithay's own test (`X11Surface`'s `DndFocus::enter`).
#[cfg(feature = "xwayland")]
fn from_x<S: Source>(window: &X11Surface, source: &S) -> bool {
    window
        .xwm_id()
        .is_none_or(|xwm| source.is_client_local(&xwm))
}

/// A drag's target: the surface's `wl_data_device`, or -- for an X window
/// and a drag from Wayland -- Smithay's XDND target (see the module doc).
///
/// **Drags from X stay on the surface's path, for now.** Over an X window,
/// Smithay's X target unmaps the window manager's proxy so the X source
/// can drop on the window itself, and maps it back when the drag leaves --
/// but at the pinned rev (and upstream) that remap is never flushed to the
/// X server: the proxy stays unmapped, and an X drag that has crossed an X
/// window -- every one does, starting over its own -- can no longer drop
/// on a Wayland window, which works today. Measured, not assumed:
/// `tests/drop.rs` found no proxy over the Wayland window until an
/// unrelated window-manager request flushed the connection. So the X target
/// is reached only through an X offer, which only a drag from Wayland gets;
/// its offer-less branches -- the remap, and finishing an X drag -- are
/// unreachable, and a drag from X takes exactly the `WlSurface` path it
/// always took. The fork commit that flushes the remap turns the rest on
/// (`docs/backlog/protocols/xwayland-pointer-focus-x11.md`).
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
            Self::X11 { window, .. } if !from_x(window, &*source) => {
                DndFocus::enter(window, data, dh, source, seat, location, serial)
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
            (Self::X11 { window, .. }, Some(PointerOffer::X11(offer))) => {
                DndFocus::motion(window, data, Some(offer), seat, location, time);
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
            (Self::X11 { window, .. }, Some(PointerOffer::X11(offer))) => {
                DndFocus::leave(window, data, Some(offer), seat);
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
            (Self::X11 { window, .. }, Some(PointerOffer::X11(offer))) => {
                DndFocus::drop(window, data, Some(offer), seat);
            }
            (focus, offer) => {
                let offer = offer.and_then(PointerOffer::as_surface);
                DndFocus::drop(focus.surface(), data, offer, seat);
            }
        }
    }
}
