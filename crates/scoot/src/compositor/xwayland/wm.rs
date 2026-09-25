//! The XWayland protocol halves Smithay dispatches into: the association
//! protocol, the window manager, and the keyboard-grab refusal.
//!
//! Each `XwmHandler` method is a thin dispatch into the module that owns
//! the policy: `manage.rs` (managed windows entering, changing and leaving
//! the core), `unmanaged.rs` (override-redirect windows, drawn but never
//! laid out) and `focus.rs` (the focus gate, and the one place an X client
//! can ask for focus).

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::xwayland_keyboard_grab::XWaylandKeyboardGrabHandler;
use smithay::wayland::xwayland_shell::{XWaylandShellHandler, XWaylandShellState};
use smithay::xwayland::xwm::{Reorder, ResizeEdge, WmWindowProperty, X11Window, XwmId};
use smithay::xwayland::{X11Surface, X11Wm, XwmHandler};

use super::super::State;
use super::super::keyboard_focus::KeyboardFocus;

/// `xwayland_shell_v1`: pairs each X window with the `wl_surface` XWayland
/// draws it into. Smithay records the pair itself; what scoot adds is the
/// keyboard: a window focused before its surface existed had no surface to
/// hand the keyboard to (see `State::window_keyboard_focus`), so the pairing
/// re-derives it.
impl XWaylandShellHandler for State {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    fn surface_associated(&mut self, _xwm: XwmId, _surface: WlSurface, window: X11Surface) {
        self.x11_surface_associated(&window);
    }
}

impl XwmHandler for State {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        // Callbacks originate from the stored manager's own event handling,
        // so it is always there when one fires -- the way anvil reads its
        // `Option` too. An `expect`, not an `Option` return, because the
        // trait gives no other shape and inventing a dummy would be worse.
        self.xwm
            .as_mut()
            .expect("an XWM callback fired without a running X server")
    }

    fn new_window(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::debug!(id = window.window_id(), "X11 window created");
    }

    fn new_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        tracing::debug!(
            id = window.window_id(),
            "X11 override-redirect window created (unmanaged)"
        );
    }

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.map_x11_window(window);
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.map_x11_unmanaged(window);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.forget_x11_window(&window, "unmapped");
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        // Usually a no-op by now: an X window unmaps before it is destroyed.
        // Kept for the order that skips the unmap (a client that destroys a
        // mapped window outright), and matched by id, since Smithay marks
        // the surface dead before this runs (see `State::id_of_x11`).
        self.forget_x11_window(&window, "destroyed");
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        reorder: Option<Reorder>,
    ) {
        tracing::trace!(
            id = window.window_id(),
            ?x,
            ?y,
            ?w,
            ?h,
            ?reorder,
            "X11 configure request"
        );
        self.x11_configure_request(&window, x, y, w, h);
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        _geometry: Rectangle<i32, Logical>,
        _above: Option<X11Window>,
    ) {
        // Only an override-redirect window moves itself; Smithay has already
        // recorded where. A managed window's notifies echo scoot's own
        // configures, which already went through `apply()`.
        if window.is_override_redirect() {
            self.x11_unmanaged_moved();
        }
    }

    fn property_notify(&mut self, _xwm: XwmId, window: X11Surface, property: WmWindowProperty) {
        match property {
            WmWindowProperty::Title
            | WmWindowProperty::Class
            | WmWindowProperty::NormalHints
            | WmWindowProperty::TransientFor => self.x11_properties_changed(&window),
            _ => {}
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_fullscreen_request(&window, true);
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_fullscreen_request(&window, false);
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        button: u32,
        resize_edge: ResizeEdge,
    ) {
        // `_NET_WM_MOVERESIZE`: not honoured yet (see the module doc). The
        // modifier drag (`[floating] modifier`) moves and resizes a floating
        // X window like any other.
        tracing::debug!(
            id = window.window_id(),
            button,
            ?resize_edge,
            "X11 resize request refused (client-initiated moves and resizes are not honoured yet)"
        );
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, button: u32) {
        tracing::debug!(
            id = window.window_id(),
            button,
            "X11 move request refused (client-initiated moves and resizes are not honoured yet)"
        );
    }

    /// `_NET_ACTIVE_WINDOW`: a request through the focus gate, never a
    /// command (see `focus.rs`).
    fn active_window_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        _timestamp: u32,
        _currently_active_window: Option<X11Surface>,
    ) {
        // `currently_active_window` is deliberately unread: it is a field of
        // the client's message, so it proves nothing about who sent it.
        self.x11_activation_request(&window);
    }

    fn disconnected(&mut self, _xwm: XwmId) {
        // The post-`READY` death signal (the pre-`READY` one is
        // `XWaylandEvent::Error` -- see `xwayland/mod.rs`): the server is
        // gone, the session is not. `warn!`, not `debug!`: an operator whose
        // X apps just died needs this line, and it fires once per session
        // death, not per event. `xdisplay` is deliberately *not* cleared
        // (see `xwayland/mod.rs`'s staleness note).
        tracing::warn!(
            display = ?self.xdisplay,
            "XWayland connection lost; the session continues Wayland-only (restart for X11)"
        );
        // A dead server sends no unmap or destroy for the windows it had
        // mapped, so without this they would stay in the layout as empty
        // columns and stale taskbar entries. `self.xwm` itself is left in
        // place: this runs inside its own event callback.
        self.sweep_x11_windows();
    }
}

/// `zwp_xwayland_keyboard_grab_manager_v1`: refused, for every surface.
/// XWayland asks for this grab when an X client grabs the keyboard
/// (`XGrabKeyboard`: a VM viewer, a remote-desktop client), and granting it
/// would let an X client take every keystroke from the Wayland session --
/// the inverse of the focus gate. `None` means Smithay creates nothing, so a
/// grab request is a silent no-op; the X client keeps its grab *inside* the
/// X server, which only sees keys while scoot has focused one of its
/// windows anyway.
impl XWaylandKeyboardGrabHandler for State {
    fn keyboard_focus_for_xsurface(&self, _surface: &WlSurface) -> Option<KeyboardFocus> {
        None
    }
}
