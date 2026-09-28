//! `_NET_WM_MOVERESIZE`: an X app's own titlebar and border drags (see
//! `xwayland/moveresize.rs`). A real X client presses the way a toolkit
//! does -- the press goes through scoot, XWayland delivers it, the client
//! gives up its implicit grab and sends the request -- and the tests read
//! what scoot's pointer and layout did with it.
//!
//! A refused request changes nothing, which is also what an unprocessed one
//! looks like; so every refusal is asserted only after a barrier: the
//! requesting connection retitles a window after sending, and the title
//! reaching the core proves the request before it was handled (the X
//! server hands the window manager one connection's traffic in order).

use scoot_core::{Rect, WindowId};
use scoot_ipc::PointerButton;
use smithay::input::pointer::ClickGrab;
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::xproto::Window as XWindow;

use super::dnd::taken_over;
use super::live::{BLUE, Live, RED, live};
use super::peer::{Ack, Step};
use super::x11::{
    MOVERESIZE_CANCEL, MOVERESIZE_MOVE, MOVERESIZE_MOVE_KEYBOARD, MOVERESIZE_SIZE_BOTTOMRIGHT,
    MOVERESIZE_SIZE_KEYBOARD, Props, XClient, eventually,
};
use crate::compositor::State;

/// The X button a left press arrives as.
const X_LEFT: u32 = 1;
/// The X button a right press arrives as.
const X_RIGHT: u32 = 3;

/// Maps a floating X window (a dialog) of `x`'s, waits until it is laid
/// out, drawn and paired, and selects its button events.
fn floating(live: &mut Live, x: Option<&XClient>, pixel: u32) -> (XWindow, WindowId) {
    let props = Props {
        dialog: true,
        ..Props::new(pixel)
    };
    let xid = x.unwrap_or(&live.x).map(&props);
    let id = live.managed(xid);
    assert!(live.placement(id).floating, "a dialog did not float");
    x.unwrap_or(&live.x).select_buttons(xid);
    (xid, id)
}

/// Where a titlebar press lands on `rect`: near its top-left.
fn titlebar(rect: Rect) -> (i32, i32) {
    (rect.x + 20, rect.y + 10)
}

fn press(live: &mut Live, (x, y): (i32, i32), button: PointerButton) {
    live.fixture.state.pointer_move(f64::from(x), f64::from(y));
    live.fixture.state.pointer_button(button, true);
    live.drain();
}

fn release(live: &mut Live, button: PointerButton) {
    live.fixture.state.pointer_button(button, false);
    live.drain();
}

/// Waits until `x`'s ButtonPress on `xid` arrives: the toolkit's cue.
fn x_saw_press(live: &mut Live, x: Option<&XClient>, xid: XWindow) {
    let x = x.unwrap_or(&live.x);
    eventually(&mut live.fixture, "the X client's ButtonPress", |_| {
        x.drain()
            .iter()
            .any(|event| matches!(event, XEvent::ButtonPress(press) if press.event == xid))
    });
}

/// What a toolkit does once a titlebar press passes its drag threshold:
/// gives up its grab and asks the window manager to take the drag over.
fn ask(x: &XClient, xid: XWindow, at: (i32, i32), direction: u32, button: u32) {
    x.ungrab_pointer();
    x.request_moveresize(xid, at, direction, button);
}

/// The barrier (see the module doc): `x` retitles `xid`, and this waits
/// until the core has the title.
fn barrier(live: &mut Live, x: Option<&XClient>, xid: XWindow, id: WindowId, title: &'static str) {
    x.unwrap_or(&live.x).retitle(xid, title);
    eventually(&mut live.fixture, "the barrier title", |fixture| {
        fixture
            .state
            .world
            .window_info(id)
            .is_some_and(|info| info.title == title)
    });
}

fn still_a_plain_press(state: &State) -> bool {
    state
        .seat
        .get_pointer()
        .and_then(|pointer| {
            pointer.with_grab(|_, grab| grab.downcast_ref::<ClickGrab<State>>().is_some())
        })
        .unwrap_or(false)
}

fn grabbed(state: &State) -> bool {
    state
        .seat
        .get_pointer()
        .is_some_and(|pointer| pointer.is_grabbed())
}

/// A press on a floating X window's titlebar, the toolkit asking for a
/// move: scoot takes the drag, the window follows the pointer, and the
/// release ends it -- with the X server told where the window now is, and
/// no button left held on the X side.
#[test]
fn a_titlebar_drag_moves_a_floating_x_window() {
    let Some(mut live) = live("a_titlebar_drag_moves_a_floating_x_window") else {
        return;
    };
    let (xid, id) = floating(&mut live, None, RED);
    let before = live.placement(id).rect;
    let at = titlebar(before);
    press(&mut live, at, PointerButton::Left);
    x_saw_press(&mut live, None, xid);
    ask(&live.x, xid, at, MOVERESIZE_MOVE, X_LEFT);
    eventually(&mut live.fixture, "the drag starting", |fixture| {
        fixture.state.floating_grab_window() == Some(id)
    });
    live.fixture
        .state
        .pointer_move(f64::from(at.0 - 30), f64::from(at.1 + 25));
    live.drain();
    let moved = Rect::new(before.x - 30, before.y + 25, before.w, before.h);
    assert_eq!(live.placement(id).rect, moved, "the window did not follow");
    release(&mut live, PointerButton::Left);
    assert_eq!(live.fixture.state.floating_grab_window(), None);
    assert!(!grabbed(&live.fixture.state), "a grab outlived the release");
    // Kept where it was dropped, and the X server agrees -- a client
    // positions its dialogs from its own root coordinates.
    live.fixture
        .state
        .pointer_move(f64::from(at.0), f64::from(at.1));
    live.drain();
    assert_eq!(live.placement(id).rect, moved, "the drop was not kept");
    let (x, y, _, _) = live.x.root_geometry(xid);
    assert_eq!((x, y), (moved.x, moved.y), "the X server has it elsewhere");
    assert_eq!(live.x.buttons_down(), 0, "a button stayed held X-side");
    // And the window takes an ordinary click again, where it now is.
    live.x.drain();
    let at = titlebar(moved);
    press(&mut live, at, PointerButton::Left);
    x_saw_press(&mut live, None, xid);
    release(&mut live, PointerButton::Left);
}

/// A border drag: `_NET_WM_MOVERESIZE` with an edge resizes a floating X
/// window from that edge, and the X server has the new size once the drag
/// ends (an X window is configured when the drag settles, as under the
/// modifier drag).
#[test]
fn a_border_drag_resizes_a_floating_x_window() {
    let Some(mut live) = live("a_border_drag_resizes_a_floating_x_window") else {
        return;
    };
    let (xid, id) = floating(&mut live, None, RED);
    let before = live.placement(id).rect;
    let at = (before.x + before.w - 3, before.y + before.h - 3);
    press(&mut live, at, PointerButton::Left);
    x_saw_press(&mut live, None, xid);
    ask(&live.x, xid, at, MOVERESIZE_SIZE_BOTTOMRIGHT, X_LEFT);
    eventually(&mut live.fixture, "the resize starting", |fixture| {
        fixture.state.floating_grab_window() == Some(id)
    });
    live.fixture
        .state
        .pointer_move(f64::from(at.0 + 40), f64::from(at.1 + 30));
    live.drain();
    release(&mut live, PointerButton::Left);
    assert_eq!(live.fixture.state.floating_grab_window(), None);
    let after = live.placement(id).rect;
    assert_eq!(
        (after.x, after.y, after.w, after.h),
        (before.x, before.y, before.w + 40, before.h + 30),
        "not resized from the bottom-right corner"
    );
    live.drain();
    let (_, _, w, h) = live.x.root_geometry(xid);
    assert_eq!(
        (w, h),
        (
            u32::try_from(after.w).expect("positive"),
            u32::try_from(after.h).expect("positive")
        ),
        "the X server has another size"
    );
}

/// A tiled X window's request is ignored, as a tiled Wayland window's is:
/// the strip places tiled windows, and the press stays the client's own --
/// its release still reaches it.
#[test]
fn a_tiled_x_window_s_move_request_is_ignored() {
    let Some(mut live) = live("a_tiled_x_window_s_move_request_is_ignored") else {
        return;
    };
    let xid = live.x.map(&Props::new(RED));
    let id = live.managed(xid);
    live.x.select_buttons(xid);
    let column = live.placement(id).rect;
    assert!(!live.placement(id).floating);
    let at = titlebar(column);
    press(&mut live, at, PointerButton::Left);
    x_saw_press(&mut live, None, xid);
    ask(&live.x, xid, at, MOVERESIZE_MOVE, X_LEFT);
    barrier(&mut live, None, xid, id, "tiled barrier");
    assert_eq!(live.fixture.state.floating_grab_window(), None);
    assert!(
        still_a_plain_press(&live.fixture.state),
        "the press stopped being the client's"
    );
    live.fixture
        .state
        .pointer_move(f64::from(at.0 + 40), f64::from(at.1 + 40));
    live.drain();
    assert_eq!(live.placement(id).rect, column);
    live.x.drain();
    release(&mut live, PointerButton::Left);
    let events = live.x.drain();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, XEvent::ButtonRelease(release) if release.event == xid)),
        "the release did not reach the X client: {events:?}"
    );
}

/// No button held -- never pressed, or already released by the time the
/// request is handled (the order a slow client produces): refused. A
/// background X client cannot make scoot capture the pointer.
#[test]
fn a_move_request_with_no_held_button_is_refused() {
    let Some(mut live) = live("a_move_request_with_no_held_button_is_refused") else {
        return;
    };
    let (xid, id) = floating(&mut live, None, RED);
    let before = live.placement(id).rect;
    let at = titlebar(before);
    live.fixture
        .state
        .pointer_move(f64::from(at.0), f64::from(at.1));
    live.drain();
    live.x.request_moveresize(xid, at, MOVERESIZE_MOVE, X_LEFT);
    barrier(&mut live, None, xid, id, "never pressed");
    assert_eq!(live.fixture.state.floating_grab_window(), None);
    assert!(!grabbed(&live.fixture.state));
    // Released before the request lands.
    press(&mut live, at, PointerButton::Left);
    x_saw_press(&mut live, None, xid);
    release(&mut live, PointerButton::Left);
    ask(&live.x, xid, at, MOVERESIZE_MOVE, X_LEFT);
    barrier(&mut live, None, xid, id, "already released");
    assert_eq!(live.fixture.state.floating_grab_window(), None);
    assert!(!grabbed(&live.fixture.state));
    live.fixture
        .state
        .pointer_move(f64::from(at.0 + 50), f64::from(at.1 + 50));
    live.drain();
    assert_eq!(live.placement(id).rect, before, "the window moved");
}

/// The press is held on one X client's window and *another* X client asks
/// to move its own floating window: refused. A drag rides only on a press
/// its own client was given.
#[test]
fn a_move_request_on_another_x_client_s_press_is_refused() {
    let Some(mut live) = live("a_move_request_on_another_x_client_s_press_is_refused") else {
        return;
    };
    let (pressed, pressed_id) = floating(&mut live, None, RED);
    let stranger = XClient::connect(live.display);
    let (theirs, theirs_id) = floating(&mut live, Some(&stranger), BLUE);
    // The stranger's dialog was mapped last, so it is on top: move the
    // pressed one clear of it first.
    let rect = live.placement(pressed_id).rect;
    live.fixture.state.act(scoot_core::Action::MoveFloating {
        id: pressed_id,
        x: 0,
        y: 0,
    });
    live.drain();
    let rect = Rect::new(0, 0, rect.w, rect.h);
    let at = titlebar(rect);
    press(&mut live, at, PointerButton::Left);
    x_saw_press(&mut live, None, pressed);
    let theirs_before = live.placement(theirs_id).rect;
    stranger.request_moveresize(theirs, at, MOVERESIZE_MOVE, X_LEFT);
    barrier(&mut live, Some(&stranger), theirs, theirs_id, "stranger");
    assert_eq!(live.fixture.state.floating_grab_window(), None);
    assert!(still_a_plain_press(&live.fixture.state));
    live.fixture
        .state
        .pointer_move(f64::from(at.0 + 30), f64::from(at.1 + 30));
    live.drain();
    assert_eq!(live.placement(theirs_id).rect, theirs_before);
    release(&mut live, PointerButton::Left);
}

/// The press is held on a *Wayland* window and an X client asks to move
/// its floating window: refused -- the X form of "another client's press".
#[test]
fn a_move_request_riding_a_press_on_a_wayland_window_is_refused() {
    let Some(mut live) = live("a_move_request_riding_a_press_on_a_wayland_window_is_refused")
    else {
        return;
    };
    let (xid, id) = floating(&mut live, None, RED);
    let wayland = live.map_peer("wayland");
    let column = live.placement(wayland).rect;
    let before = live.placement(id).rect;
    // Clear of the dialog, which floats over the column.
    let at = (column.x + 5, column.y + 5);
    assert!(
        at.0 < before.x || at.1 < before.y,
        "the press point is under the dialog"
    );
    press(&mut live, at, PointerButton::Left);
    assert!(still_a_plain_press(&live.fixture.state));
    live.x.request_moveresize(xid, at, MOVERESIZE_MOVE, X_LEFT);
    barrier(&mut live, None, xid, id, "wayland press");
    assert_eq!(live.fixture.state.floating_grab_window(), None);
    assert!(
        still_a_plain_press(&live.fixture.state),
        "an X client took over a press on a Wayland window"
    );
    release(&mut live, PointerButton::Left);
}

/// The request names a button other than the one held: refused. `0` (no
/// button named -- what some toolkits send) rides whichever is held.
#[test]
fn a_move_request_naming_another_button_is_refused() {
    let Some(mut live) = live("a_move_request_naming_another_button_is_refused") else {
        return;
    };
    let (xid, id) = floating(&mut live, None, RED);
    let at = titlebar(live.placement(id).rect);
    press(&mut live, at, PointerButton::Left);
    x_saw_press(&mut live, None, xid);
    ask(&live.x, xid, at, MOVERESIZE_MOVE, X_RIGHT);
    barrier(&mut live, None, xid, id, "wrong button");
    assert_eq!(live.fixture.state.floating_grab_window(), None);
    assert!(still_a_plain_press(&live.fixture.state));
    // An unnamed button is the held one.
    live.x.request_moveresize(xid, at, MOVERESIZE_MOVE, 0);
    eventually(&mut live.fixture, "the unnamed-button drag", |fixture| {
        fixture.state.floating_grab_window() == Some(id)
    });
    release(&mut live, PointerButton::Left);
    assert_eq!(live.fixture.state.floating_grab_window(), None);
    assert!(!grabbed(&live.fixture.state));
}

/// While another grab holds the pointer -- here the X client's own
/// drag-and-drop, started from the very press -- a move request is refused:
/// only a plain held press can be taken over.
#[test]
fn a_move_request_during_another_grab_is_refused() {
    let Some(mut live) = live("a_move_request_during_another_grab_is_refused") else {
        return;
    };
    let (xid, id) = floating(&mut live, None, RED);
    let at = titlebar(live.placement(id).rect);
    press(&mut live, at, PointerButton::Left);
    x_saw_press(&mut live, None, xid);
    live.x.take_selection("XdndSelection");
    live.drain();
    assert!(taken_over(&live.fixture.state), "no drag started");
    live.x.request_moveresize(xid, at, MOVERESIZE_MOVE, X_LEFT);
    barrier(&mut live, None, xid, id, "during a drag");
    assert_eq!(live.fixture.state.floating_grab_window(), None);
    assert!(taken_over(&live.fixture.state), "the drag was replaced");
    release(&mut live, PointerButton::Left);
    assert!(!grabbed(&live.fixture.state));
}

/// Under the session lock: refused. (The lock also drops every grab and
/// takes the pointer, so no X press is held to ride on; the gate refuses a
/// locked session before it looks -- `the_gate_refuses_a_locked_session`.)
#[test]
fn a_move_request_under_the_lock_is_refused() {
    let Some(mut live) = live("a_move_request_under_the_lock_is_refused") else {
        return;
    };
    let (xid, id) = floating(&mut live, None, RED);
    let at = titlebar(live.placement(id).rect);
    assert!(matches!(live.fixture.run(Step::Lock), Ack::Done));
    eventually(&mut live.fixture, "the session locking", |fixture| {
        fixture.state.session_lock.is_locked()
    });
    press(&mut live, at, PointerButton::Left);
    live.x.request_moveresize(xid, at, MOVERESIZE_MOVE, X_LEFT);
    barrier(&mut live, None, xid, id, "locked");
    assert_eq!(live.fixture.state.floating_grab_window(), None);
    release(&mut live, PointerButton::Left);
}

/// The gate's lock branch on its own: a press genuinely held on the
/// requesting client's window, which the gate accepts unlocked, is refused
/// once `locked` -- the one condition the live test above cannot isolate
/// (locking drops the press first).
#[test]
fn the_gate_refuses_a_locked_session() {
    use super::super::moveresize::{Refusal, x11_moveresize_gate};

    let Some(mut live) = live("the_gate_refuses_a_locked_session") else {
        return;
    };
    let (xid, id) = floating(&mut live, None, RED);
    let at = titlebar(live.placement(id).rect);
    press(&mut live, at, PointerButton::Left);
    x_saw_press(&mut live, None, xid);
    let pointer = live.fixture.state.seat.get_pointer().expect("a pointer");
    let held = crate::compositor::floating::grab::held_click(&pointer, None);
    assert!(held.is_some(), "no press held");
    assert!(x11_moveresize_gate(false, held.clone(), xid, X_LEFT).is_ok());
    assert!(matches!(
        x11_moveresize_gate(true, held, xid, X_LEFT),
        Err(Refusal::Locked)
    ));
    release(&mut live, PointerButton::Left);
}

/// Keyboard moves and resizes (directions 9 and 10) start nothing, even
/// with a press held: scoot has no keyboard move mode (see
/// `xwayland/moveresize.rs`).
#[test]
fn a_keyboard_move_or_resize_is_refused() {
    let Some(mut live) = live("a_keyboard_move_or_resize_is_refused") else {
        return;
    };
    let (xid, id) = floating(&mut live, None, RED);
    let at = titlebar(live.placement(id).rect);
    press(&mut live, at, PointerButton::Left);
    x_saw_press(&mut live, None, xid);
    live.x.ungrab_pointer();
    for direction in [MOVERESIZE_MOVE_KEYBOARD, MOVERESIZE_SIZE_KEYBOARD] {
        live.x.request_moveresize(xid, at, direction, X_LEFT);
    }
    barrier(&mut live, None, xid, id, "keyboard");
    assert_eq!(live.fixture.state.floating_grab_window(), None);
    assert!(still_a_plain_press(&live.fixture.state));
    release(&mut live, PointerButton::Left);
}

/// `_NET_WM_MOVERESIZE_CANCEL` does not reach scoot at the pinned Smithay
/// (its window manager drops direction 11), so a drag it would cancel runs
/// on -- and the release still ends it, which is what keeps it from ever
/// sticking. Pinned so a Smithay that starts delivering it is noticed: then
/// this should become "the cancel ends the drag".
#[test]
fn a_cancel_is_not_delivered_and_the_release_still_ends_the_drag() {
    let Some(mut live) = live("a_cancel_is_not_delivered_and_the_release_still_ends_the_drag")
    else {
        return;
    };
    let (xid, id) = floating(&mut live, None, RED);
    let at = titlebar(live.placement(id).rect);
    press(&mut live, at, PointerButton::Left);
    x_saw_press(&mut live, None, xid);
    ask(&live.x, xid, at, MOVERESIZE_MOVE, X_LEFT);
    eventually(&mut live.fixture, "the drag starting", |fixture| {
        fixture.state.floating_grab_window() == Some(id)
    });
    live.x
        .request_moveresize(xid, at, MOVERESIZE_CANCEL, X_LEFT);
    barrier(&mut live, None, xid, id, "cancel");
    assert_eq!(
        live.fixture.state.floating_grab_window(),
        Some(id),
        "the cancel now reaches scoot: honour it (xwayland/moveresize.rs) and flip this test"
    );
    release(&mut live, PointerButton::Left);
    assert_eq!(live.fixture.state.floating_grab_window(), None);
    assert!(!grabbed(&live.fixture.state));
}

/// The window unmaps mid-drag (the app closing its dialog): the drag ends
/// with it, and the release that follows finds no grab to end.
#[test]
fn an_x_window_unmapping_mid_drag_ends_the_drag() {
    let Some(mut live) = live("an_x_window_unmapping_mid_drag_ends_the_drag") else {
        return;
    };
    let (xid, id) = floating(&mut live, None, RED);
    let at = titlebar(live.placement(id).rect);
    press(&mut live, at, PointerButton::Left);
    x_saw_press(&mut live, None, xid);
    ask(&live.x, xid, at, MOVERESIZE_MOVE, X_LEFT);
    eventually(&mut live.fixture, "the drag starting", |fixture| {
        fixture.state.floating_grab_window() == Some(id)
    });
    live.x.unmap(xid);
    eventually(&mut live.fixture, "the drag ending", |fixture| {
        fixture.state.floating_grab_window().is_none()
    });
    live.fixture
        .state
        .pointer_move(f64::from(at.0 + 30), f64::from(at.1 + 30));
    release(&mut live, PointerButton::Left);
    assert!(!grabbed(&live.fixture.state));
}

/// A known limit, pinned so it is not mistaken for a guarantee: any X
/// client can send `_NET_WM_MOVERESIZE` naming *any* window, and the window
/// manager learns only the window. So a stranger naming the very window the
/// press is held on passes the same-client check, and scoot drags that
/// window until the release -- the same limit as an X drag's owner (see
/// `xwayland/dnd.rs`). What the gate does hold is that the press is real,
/// still held, and on that window's client: nothing captures the pointer
/// without one, and a press on a Wayland window or another X client's
/// window is never taken.
#[test]
fn a_stranger_naming_the_pressed_x_window_moves_it() {
    let Some(mut live) = live("a_stranger_naming_the_pressed_x_window_moves_it") else {
        return;
    };
    let (xid, id) = floating(&mut live, None, RED);
    let at = titlebar(live.placement(id).rect);
    press(&mut live, at, PointerButton::Left);
    x_saw_press(&mut live, None, xid);
    let stranger = XClient::connect(live.display);
    stranger.request_moveresize(xid, at, MOVERESIZE_MOVE, X_LEFT);
    eventually(&mut live.fixture, "the stranger's drag", |fixture| {
        fixture.state.floating_grab_window() == Some(id)
    });
    release(&mut live, PointerButton::Left);
    assert!(!grabbed(&live.fixture.state));
}
