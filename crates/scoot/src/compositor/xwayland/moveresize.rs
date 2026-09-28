//! `_NET_WM_MOVERESIZE`: an X app's own titlebar and border drags -- the X
//! form of `xdg_toplevel.move` / `.resize` (see `floating/grab.rs`).
//!
//! An X app that draws its own titlebar (GTK with client-side decorations
//! over X, Chromium, Electron) takes the press itself, and once the
//! pointer passes its drag threshold gives up its pointer grab and asks
//! the window manager to take the drag over. Smithay's window manager
//! decodes the request into `XwmHandler::move_request` / `resize_request`;
//! from there it goes where an xdg request goes, and starts the same
//! floating grab, under the same rules.
//!
//! **The gate, restated for X.** An xdg request carries the serial of the
//! press it rides on; X has none, so the press is found the way the xdg
//! path confirms it: the pointer's grab must be Smithay's implicit click
//! grab -- a button held right now, not a popup's, a drag-and-drop's or a
//! modifier drag's grab -- and the surface that press went to must be an X
//! window of the requesting window's X client (the client bits of the two
//! window ids, which the X server allocates per connection:
//! `focus::same_x_client`). When the request names a button (`data[3]`),
//! it must be the held one; `0` names none and rides whichever is held.
//! Nothing while the session is locked. The window must be managed,
//! floating and on screen: a tiled or fullscreen window's request is
//! ignored, as an xdg one is, and its press stays its own.
//!
//! So a background X client cannot capture the pointer: with no press
//! held, or one held on a Wayland window or on another X client's window,
//! the request is refused (logged at debug) and nothing happens. A request
//! handled after its button was released finds no click grab either: the
//! release reaches scoot before XWayland, so an X client cannot have seen a
//! release scoot has not. One exception, not prevented: a request carries no
//! serial, so one still queued when the user releases and presses again in
//! the same X client within one stalled loop iteration rides the new press
//! (as a drag, ended by that press's release). The drag never sticks: its
//! release comes to scoot's grab, which ends there, as every floating drag
//! does.
//!
//! **XWayland's own grab.** The press also gave the X client an implicit X
//! pointer grab, which EWMH has it release before asking. scoot's grab
//! clears Wayland pointer focus, so XWayland sees the pointer leave and
//! none of the drag's motion or its release; when the drag ends the pointer
//! enters again, and the X server holds no button (measured:
//! `tests/moveresize.rs` reads the X server's button mask after a drag).
//!
//! **What an X drag does not do yet, and why.**
//!
//! - **Keyboard moves and resizes** (`_NET_WM_MOVERESIZE_MOVE_KEYBOARD`,
//!   `_SIZE_KEYBOARD`: a window menu's "Move") start nothing. Smithay's
//!   window manager drops both before any handler runs, and honouring them
//!   would need a keyboard move mode scoot does not have -- a keyboard grab
//!   taking the arrow keys from the focused window. A floating window is
//!   moved from the keyboard with scoot's own bindings and `scoot msg`
//!   instead.
//! - **`_NET_WM_MOVERESIZE_CANCEL`** is dropped by Smithay's window manager
//!   the same way, so a client cannot end a drag early; the release ends
//!   it. Nothing is lost by that (see above: the release always reaches the
//!   grab), only a client's own cancel. Honouring it needs a window-manager
//!   hook the pinned Smithay does not have.
//! - **The request's root position** (`data[0..2]`) is not read: Smithay
//!   does not pass it on, and the press's own position -- what the xdg
//!   path measures from -- is the better anchor anyway (the pointer is past
//!   the toolkit's drag threshold by the time it asks).
//!
//! **A known limit.** Any X client can send `_NET_WM_MOVERESIZE` naming any
//! window, and the window manager learns only the window, never who sent
//! it. So a stranger naming any window of the X client the press is held on
//! -- the pressed window or another of that client's -- passes the
//! same-client check, and that window is dragged until the release. X11
//! gives no way to close that -- the same limit as an X drag-and-drop's
//! owner (`dnd.rs`) -- and it cannot reach past the press: no pointer is
//! captured without one, and none held on a Wayland window or another X
//! client's window is ever taken.

use scoot_core::Edges;
use smithay::input::pointer::GrabStartData;
use smithay::utils::Serial;
use smithay::xwayland::X11Surface;
use smithay::xwayland::xwm::ResizeEdge;

use super::super::State;
use super::super::floating::grab::held_click;
use super::super::input::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT};
use super::super::pointer_focus::PointerFocus;
use super::focus::same_x_client;

/// Why a `_NET_WM_MOVERESIZE` was refused -- logged, and what the gate's
/// tests match on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    Locked,
    NoHeldPress,
    NotOnAnXWindow,
    PressedWindowGone,
    AnotherClientsPress,
    AnotherButton,
}

impl State {
    /// `XwmHandler::move_request` (`edges` `None`) and `resize_request`: see
    /// the module doc for what is honoured.
    pub(super) fn x11_moveresize_request(
        &mut self,
        window: &X11Surface,
        button: u32,
        edges: Option<Edges>,
    ) {
        let xid = window.window_id();
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let held = held_click(&pointer, None);
        let (serial, start) =
            match x11_moveresize_gate(self.session_lock.is_locked(), held, xid, button) {
                Ok(held) => held,
                Err(refusal) => {
                    tracing::debug!(
                        xid,
                        button,
                        ?edges,
                        ?refusal,
                        "refusing an X window's move/resize request"
                    );
                    return;
                }
            };
        // An override-redirect window (a menu) is none scoot manages: it
        // places itself.
        let Some(id) = self.id_of_x11(window) else {
            tracing::debug!(
                xid,
                "ignoring a move/resize request for an unmanaged X window"
            );
            return;
        };
        self.begin_client_drag(&pointer, id, &start, serial, edges);
    }
}

/// The gate (see the module doc): whether the press `held` -- the pointer's
/// click grab, if one is held (`held_click`) -- may carry a move or resize
/// of X window `requester` asked for with X button `button`. Answers the
/// press to ride on. Pure over its inputs, so each refusal is testable on
/// its own (the lock above all, which live input cannot isolate: locking
/// drops the press first).
pub(super) fn x11_moveresize_gate(
    locked: bool,
    held: Option<(Serial, GrabStartData<State>)>,
    requester: u32,
    button: u32,
) -> Result<(Serial, GrabStartData<State>), Refusal> {
    if locked {
        return Err(Refusal::Locked);
    }
    let Some((serial, start)) = held else {
        return Err(Refusal::NoHeldPress);
    };
    // The hit test names every X window -- managed or override-redirect --
    // as the X focus (`State::surface_under`), so a press on one carries the
    // window itself; anything else is a Wayland surface (or none).
    let Some((PointerFocus::X11 { window, .. }, _)) = &start.focus else {
        return Err(Refusal::NotOnAnXWindow);
    };
    // The focus is the press's, and can outlive the window: a dead one's id
    // may already be another client's.
    if !window.alive() {
        return Err(Refusal::PressedWindowGone);
    }
    if !same_x_client(window.window_id(), requester) {
        return Err(Refusal::AnotherClientsPress);
    }
    if button != 0 && x_button_code(button) != Some(start.button) {
        return Err(Refusal::AnotherButton);
    }
    Ok((serial, start))
}

/// The evdev code of X button `button`, as XWayland numbers them
/// (`xwayland-input.c`, `pointer_handle_button`): left, middle and right
/// are 1-3, 4-7 are the scroll axes (never a held button), and `BTN_SIDE`
/// onwards count up from 8. `None` for a scroll button, `0`, or one past
/// the evdev range.
fn x_button_code(button: u32) -> Option<u32> {
    const BTN_SIDE: u32 = 0x113;
    match button {
        1 => Some(BTN_LEFT),
        2 => Some(BTN_MIDDLE),
        3 => Some(BTN_RIGHT),
        8.. => BTN_SIDE.checked_add(button - 8),
        _ => None,
    }
}

/// The edges an X resize request names. Smithay decodes all eight; the
/// keyboard and cancel directions never reach a handler.
pub(super) fn x11_edges(edge: ResizeEdge) -> Edges {
    let (left, right, top, bottom) = match edge {
        ResizeEdge::Top => (false, false, true, false),
        ResizeEdge::Bottom => (false, false, false, true),
        ResizeEdge::Left => (true, false, false, false),
        ResizeEdge::Right => (false, true, false, false),
        ResizeEdge::TopLeft => (true, false, true, false),
        ResizeEdge::TopRight => (false, true, true, false),
        ResizeEdge::BottomLeft => (true, false, false, true),
        ResizeEdge::BottomRight => (false, true, false, true),
    };
    Edges {
        left,
        right,
        top,
        bottom,
    }
}

#[cfg(test)]
mod tests;
