//! Phase 4: drags that start in X -- the X form of `dnd_requested`'s serial
//! check (see `handlers.rs`).
//!
//! An X client starts a drag by taking the `XdndSelection` while a button
//! is held. Smithay's window manager then turns the held press into a
//! drag-and-drop grab of the X client's data (`xwm/dnd.rs`), carrying it
//! across the whole session: over Wayland windows it becomes a
//! `wl_data_device` offer, and releasing drops it there. Upstream, the only
//! condition is that *some* press is held -- whose, it does not ask -- so a
//! background X client could take over a press the user made on a Wayland
//! terminal (selecting text, say) and drop its own payload into whatever is
//! under the pointer at release. That hole was live on `main` since Phase 1
//! started the window manager (pinned by `tests/dnd.rs`, fail-first).
//!
//! A drag is allowed only where a Wayland drag would be: the press is a
//! real, recent button press (`interaction_serials`, the strict `contains`
//! half -- the same bar `dnd_requested` holds) delivered to XWayland, on a
//! window whose X client (the client bits of its id, which the X server
//! allocates per connection -- `focus::same_x_client`) is that of the window
//! now owning `XdndSelection`. Refused while the session is locked, and for
//! touch -- `dnd_requested` refuses touch drags too. Like a Wayland drag, the
//! press must still be within `INTERACTION_WINDOW` (10 s): a press held past
//! it cannot start a drag even while still held. The hook it rides on,
//! `XwmHandler::allow_drag`, is a scoot-sh fork addition.
//!
//! **What this protects, and what it cannot.** A press on a *Wayland*
//! surface can no longer be taken over by any X client -- the hole above,
//! closed. A press on an *X* window still can be: `SetSelectionOwner`
//! accepts any window id, so a stranger can take `XdndSelection` under the
//! very window the press landed on, and the window manager learns only the
//! window, never which client made the request (measured; pinned as a known
//! limit by `tests/dnd.rs`). X11 offers no way to close that, and inside the
//! X server any X client can already drive another's input anyway (the
//! trust note in `docs/protocols.md`).
//!
//! A refused drag leaves the press an ordinary press: the X client's own
//! XDND traffic with other X windows is X-side and untouched.
//!
//! "The same X client" means the same X *connection*, not the same process:
//! an app whose pressed window and drag source sit on two connections of its
//! own is refused. No toolkit measured does that (GTK and Qt drag from the
//! connection that owns the window); the process check `focus.rs` uses (an
//! X-Resource round trip) would cover it, if one ever turns up.

use smithay::input::Seat;
use smithay::input::dnd::GrabType;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::Serial;
use smithay::xwayland::XWaylandClientData;
use smithay::xwayland::xwm::X11Window;

use super::super::State;
use super::focus::same_x_client;

impl State {
    /// `XwmHandler::allow_drag`: whether X window `owner`'s client may turn
    /// the grab `serial` names into a drag (the module doc's rule).
    pub(super) fn x11_drag_allowed(
        &self,
        owner: X11Window,
        seat: &Seat<State>,
        serial: Serial,
        grab: GrabType,
    ) -> bool {
        let refuse = |why: &str| {
            // `warn`, like `dnd_requested`'s refusal: nothing on the wire
            // says a drag was refused, so the log is the only way to tell it
            // from a broken drag source. One line per drag attempt.
            tracing::warn!(owner, ?serial, "refusing an X drag: {why}");
            false
        };
        if self.session_lock.is_locked() {
            return refuse("the session is locked");
        }
        if seat != &self.seat {
            return refuse("it names a seat this compositor does not own");
        }
        if grab == GrabType::Touch {
            return refuse("touch drags are not supported");
        }
        let Some(start) = seat
            .get_pointer()
            .and_then(|pointer| pointer.grab_start_data())
        else {
            return refuse("no press is held");
        };
        let Some((pressed, _)) = start.focus else {
            return refuse("the press landed on no surface");
        };
        let Some(client) = pressed.client() else {
            return refuse("the pressed surface is gone");
        };
        if client.get_data::<XWaylandClientData>().is_none() {
            return refuse("the press landed on a Wayland surface");
        }
        if !self.interaction_serials.contains(serial, &client.id()) {
            return refuse("its serial is not a recent button press delivered to an X window");
        }
        let Some(window) = self.x11_window_with_surface(&pressed) else {
            return refuse("the pressed surface is no X window scoot knows");
        };
        if !same_x_client(window, owner) {
            return refuse("the press landed on another X client's window");
        }
        true
    }

    /// The X window -- managed or override-redirect -- whose `wl_surface`
    /// is `surface`. Per drag attempt: a walk over the X windows.
    fn x11_window_with_surface(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> Option<X11Window> {
        self.windows
            .values()
            .filter_map(smithay::desktop::Window::x11_surface)
            .chain(self.x11_unmanaged.iter())
            .find(|x11| x11.wl_surface().as_ref() == Some(surface))
            .map(smithay::xwayland::X11Surface::window_id)
    }
}
