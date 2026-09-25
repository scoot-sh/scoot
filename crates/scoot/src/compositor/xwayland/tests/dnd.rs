//! Phase 4: drags that start in X. An X client starts a drag by taking the
//! `XdndSelection` while a button is held; Smithay's window manager then
//! turns the held press into a drag across the whole session. These pin
//! that only the X client the press was delivered to can do that -- the X
//! form of `dnd_requested`'s serial check (see `xwayland/dnd.rs`).

use smithay::input::pointer::ClickGrab;

use super::live::{RED, live};
use super::x11::{Props, XClient};
use crate::compositor::State;

/// Whether the seat's pointer is held by the plain press grab every button
/// press starts -- i.e. no drag has taken it over.
fn still_a_plain_press(state: &State) -> bool {
    state
        .seat
        .get_pointer()
        .and_then(|pointer| {
            pointer.with_grab(|_, grab| grab.downcast_ref::<ClickGrab<State>>().is_some())
        })
        .unwrap_or(false)
}

/// Whether the seat's pointer is held by something other than the press
/// grab -- the drag the window manager starts.
fn taken_over(state: &State) -> bool {
    state
        .seat
        .get_pointer()
        .and_then(|pointer| {
            pointer.with_grab(|_, grab| grab.downcast_ref::<ClickGrab<State>>().is_none())
        })
        .unwrap_or(false)
}

fn press_at(state: &mut State, rect: scoot_core::Rect) {
    let (x, y) = (
        f64::from(rect.x + rect.w / 2),
        f64::from(rect.y + rect.h / 2),
    );
    state.pointer_move(x, y);
    state.pointer_button(scoot_ipc::PointerButton::Left, true);
}

/// The hijack: the user holds the button on a Wayland window (selecting
/// text in a terminal, say) and a background X client takes the
/// `XdndSelection`. Without the gate the window manager turns the user's
/// press into a drag of the X client's data -- the pointer leaves the
/// Wayland window, and releasing drops the X client's payload into
/// whatever is under it.
#[test]
fn a_background_x_client_cannot_turn_a_wayland_press_into_a_drag() {
    let Some(mut live) = live("a_background_x_client_cannot_turn_a_wayland_press_into_a_drag")
    else {
        return;
    };
    let wayland = live.map_peer("wayland");
    let rect = live.placement(wayland).rect;
    press_at(&mut live.fixture.state, rect);
    live.drain();
    assert!(
        still_a_plain_press(&live.fixture.state),
        "the press did not start the plain press grab"
    );
    live.x.take_selection("XdndSelection");
    live.drain();
    assert!(
        still_a_plain_press(&live.fixture.state),
        "a background X client turned a press on a Wayland window into its own drag"
    );
    live.fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Left, false);
    live.drain();
}

/// The legitimate drag: the press landed on an X window, and the X client
/// that owns that window takes the `XdndSelection` -- what every X toolkit
/// does when a drag crosses its threshold. The window manager takes the
/// pointer over for the drag.
#[test]
fn an_x_client_drags_from_its_own_pressed_window() {
    let Some(mut live) = live("an_x_client_drags_from_its_own_pressed_window") else {
        return;
    };
    let xid = live.x.map(&Props::new(RED));
    let id = live.managed(xid);
    let rect = live.placement(id).rect;
    press_at(&mut live.fixture.state, rect);
    live.drain();
    live.x.take_selection("XdndSelection");
    live.drain();
    assert!(
        taken_over(&live.fixture.state),
        "the X client could not drag from the window the press landed on"
    );
    live.fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Left, false);
    live.drain();
    assert!(
        !live
            .fixture
            .state
            .seat
            .get_pointer()
            .is_some_and(|pointer| pointer.is_grabbed()),
        "the drag outlived the release"
    );
}

/// The same press on an X window, but a *different* X client takes the
/// `XdndSelection`: refused. A drag belongs to the client the press was
/// delivered to, not to whichever X client asks first.
#[test]
fn another_x_client_cannot_hijack_a_press_on_an_x_window() {
    let Some(mut live) = live("another_x_client_cannot_hijack_a_press_on_an_x_window") else {
        return;
    };
    let xid = live.x.map(&Props::new(RED));
    let id = live.managed(xid);
    let rect = live.placement(id).rect;
    press_at(&mut live.fixture.state, rect);
    live.drain();
    let stranger = XClient::connect(live.display);
    stranger.take_selection("XdndSelection");
    live.drain();
    assert!(
        still_a_plain_press(&live.fixture.state),
        "another X client turned a press on an X window into its own drag"
    );
    live.fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Left, false);
    live.drain();
}
