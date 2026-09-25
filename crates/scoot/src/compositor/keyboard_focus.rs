//! What the seat's keyboard can be focused on: a Wayland surface, or -- in an
//! `xwayland` build -- an X11 window.
//!
//! # Why this is not simply `WlSurface`
//!
//! An X11 window does receive its keys through a `wl_surface` (XWayland's,
//! for that window), but that alone does not route them: XWayland delivers a
//! key to whichever X window holds the *X server's* input focus, and nothing
//! moves that focus but the window manager's own `SetInputFocus` (plus
//! `WM_TAKE_FOCUS` for the clients that ask for it). XWayland does not follow
//! `wl_keyboard.enter` by itself. Measured, not assumed: with the keyboard
//! handed to an X window's `wl_surface` the X focus stayed `PointerRoot`
//! (`GetInputFocus` read `0x1` before and after) and an injected key reached
//! neither of two mapped X windows; only Smithay's `X11Surface` keyboard
//! target moved it (`0x400000`, the window's own id). `PointerRoot` means
//! "whatever X window is under the X pointer", so a `WlSurface`-only focus
//! would hand keystrokes the user aimed at one X client to another -- the
//! keylogging hole this project's XWayland trust note is about, opened by
//! the compositor rather than by X11.
//!
//! So the X11 variant carries the `X11Surface` itself, and its
//! [`KeyboardTarget`] is Smithay's: `enter` sets the X focus (per the
//! window's ICCCM input model) before forwarding to the surface, `leave`
//! schedules the focus release. Every other consumer of the keyboard focus
//! -- the selection devices, text input, the popup grab -- needs only the
//! surface, which [`WaylandFocus`] gives for both variants.
//!
//! Without the `xwayland` feature this is a one-variant enum; every match on
//! it is exhaustive in both builds, so the default build's behaviour is the
//! `WlSurface` focus it always had.

use std::borrow::Cow;

use smithay::backend::input::{InputTime, KeyState};
use smithay::desktop::PopupKind;
use smithay::input::Seat;
use smithay::input::keyboard::{KeyboardTarget, KeysymHandle, ModifiersState};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{IsAlive, Serial};
use smithay::wayland::seat::WaylandFocus;
#[cfg(feature = "xwayland")]
use smithay::xwayland::X11Surface;

use super::State;

/// The seat's keyboard focus (`SeatHandler::KeyboardFocus`).
///
/// The X arm is large (an `X11Surface` carries its connection's whole atom
/// table by value), and deliberately not boxed: Smithay clones the focus
/// out of the seat on `current_focus()`, which `input.rs` asks once per key
/// event, and a box would turn that clone into a heap allocation per key.
/// Inline, the clone is a handful of reference-count bumps either way.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum KeyboardFocus {
    /// Any Wayland surface: a toplevel, a layer surface, a lock surface, a
    /// popup.
    Surface(WlSurface),
    /// A managed X11 window, with the `wl_surface` XWayland associated with
    /// it. See the module doc for why the window itself has to be here.
    ///
    /// The surface is captured when the focus is derived, and only a window
    /// that *has* one is ever focused (`State::window_keyboard_focus`; the
    /// association re-derives focus when it lands): that is what keeps the
    /// conversion to the pointer's `WlSurface` focus below total -- Smithay's
    /// popup grab requires one -- rather than a branch with nothing to
    /// return for a window XWayland has not associated yet.
    #[cfg(feature = "xwayland")]
    X11 {
        window: X11Surface,
        surface: WlSurface,
    },
}

impl KeyboardFocus {
    /// The surface keys are delivered to: the `wl_surface` itself, or the X
    /// window's associated one.
    pub fn surface(&self) -> &WlSurface {
        match self {
            Self::Surface(surface) => surface,
            #[cfg(feature = "xwayland")]
            Self::X11 { surface, .. } => surface,
        }
    }
}

impl From<WlSurface> for KeyboardFocus {
    fn from(surface: WlSurface) -> Self {
        Self::Surface(surface)
    }
}

/// What Smithay's popup grab focuses when it hands the keyboard around a
/// menu chain: always a Wayland popup, never an X window.
impl From<PopupKind> for KeyboardFocus {
    fn from(popup: PopupKind) -> Self {
        Self::Surface(popup.wl_surface().clone())
    }
}

/// The pointer's focus is a plain `WlSurface` (an X window takes pointer
/// input through its surface alone -- XWayland follows `wl_pointer.enter`,
/// unlike the keyboard), and Smithay's popup grab converts a keyboard focus
/// into one. Total by construction: every variant carries its surface.
impl From<KeyboardFocus> for WlSurface {
    fn from(focus: KeyboardFocus) -> Self {
        match focus {
            KeyboardFocus::Surface(surface) => surface,
            #[cfg(feature = "xwayland")]
            KeyboardFocus::X11 { surface, .. } => surface,
        }
    }
}

impl IsAlive for KeyboardFocus {
    fn alive(&self) -> bool {
        match self {
            Self::Surface(surface) => surface.alive(),
            #[cfg(feature = "xwayland")]
            Self::X11 { window, surface } => window.alive() && surface.alive(),
        }
    }
}

impl WaylandFocus for KeyboardFocus {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        Some(Cow::Borrowed(self.surface()))
    }

    fn same_client_as(&self, object_id: &ObjectId) -> bool {
        self.surface().id().same_client_as(object_id)
    }
}

impl KeyboardTarget<State> for KeyboardFocus {
    fn enter(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        keys: Vec<KeysymHandle<'_>>,
        serial: Serial,
    ) {
        match self {
            Self::Surface(surface) => KeyboardTarget::enter(surface, seat, data, keys, serial),
            #[cfg(feature = "xwayland")]
            Self::X11 { window, .. } => KeyboardTarget::enter(window, seat, data, keys, serial),
        }
    }

    fn leave(&self, seat: &Seat<State>, data: &mut State, serial: Serial) {
        match self {
            Self::Surface(surface) => KeyboardTarget::leave(surface, seat, data, serial),
            #[cfg(feature = "xwayland")]
            Self::X11 { window, .. } => KeyboardTarget::leave(window, seat, data, serial),
        }
    }

    fn key(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: InputTime,
    ) {
        match self {
            Self::Surface(surface) => {
                KeyboardTarget::key(surface, seat, data, key, state, serial, time);
            }
            #[cfg(feature = "xwayland")]
            Self::X11 { window, .. } => {
                KeyboardTarget::key(window, seat, data, key, state, serial, time)
            }
        }
    }

    fn modifiers(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        modifiers: ModifiersState,
        serial: Serial,
    ) {
        match self {
            Self::Surface(surface) => {
                KeyboardTarget::modifiers(surface, seat, data, modifiers, serial);
            }
            #[cfg(feature = "xwayland")]
            Self::X11 { window, .. } => {
                KeyboardTarget::modifiers(window, seat, data, modifiers, serial)
            }
        }
    }
}
