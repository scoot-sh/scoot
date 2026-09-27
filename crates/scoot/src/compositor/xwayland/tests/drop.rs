//! Drops onto X windows (`pointer_focus.rs`'s X variant): a drag from
//! Wayland reaches an X window through the window manager's XDND side, and
//! a drag from X finds the real X window under the pointer -- not the
//! window manager's full-screen proxy -- so it can drop there itself. Each
//! of these failed on the plain `WlSurface` pointer focus, where XWayland
//! (which binds no `wl_data_device`) was offered the drag and nothing
//! answered.
//!
//! The X side of each is what toolkits do: a source looks for its target
//! under the X pointer and speaks XDND to it; a target answers
//! `XdndStatus`, and on `XdndDrop` converts `XdndSelection`.
//!
//! Drags from X need Smithay's X target to unmap the window manager's proxy
//! over X windows and map it back as the drag leaves them. The remap is
//! flushed only by the scoot-sh/smithay fork's `6e6fe896`;
//! `an_x_drag_crossing_an_x_window_still_finds_the_proxy_over_wayland` and
//! `an_x_drag_whose_hovered_window_closes_finds_the_proxy_again` fail
//! without it ("no window" over the Wayland window or the desktop).

use std::sync::Arc;

use scoot_core::{Placement, Rect};
use scoot_ipc::PointerButton;
use x11rb::protocol::xproto::Window as XWindow;

use super::dnd::{press_at, taken_over};
use super::live::{BLUE, Fixture, Live, RED, live};
use super::peer::{Ack, ClipStep, Step};
use super::x11::{Props, XClient};
use super::xdnd::{Inbox, PROXY_NAME, XDND_VERSION, packed};
use crate::compositor::State;

const MIME: &str = "text/plain;charset=utf-8";

fn centre(rect: Rect) -> (f64, f64) {
    (
        f64::from(rect.x + rect.w / 2),
        f64::from(rect.y + rect.h / 2),
    )
}

fn grabbed(state: &State) -> bool {
    state
        .seat
        .get_pointer()
        .is_some_and(|pointer| pointer.is_grabbed())
}

/// A few settles, as `Live::drain` does, for when the X client is borrowed
/// apart from the fixture.
fn settle(fixture: &mut Fixture) {
    for _ in 0..20 {
        fixture.settle();
    }
}

fn visible(what: &str, placement: &Placement) {
    assert!(
        placement.visible && placement.rect.w > 0 && placement.rect.h > 0,
        "the {what} is not on screen: {placement:?}"
    );
}

/// Moves the pointer to `(x, y)` and answers the top-level X window the X
/// server then has under its pointer, having checked the X pointer really
/// is there (XWayland learns it from the drag's `wl_pointer` motion).
fn move_and_look(live: &mut Live, (x, y): (f64, f64)) -> XWindow {
    live.fixture.state.pointer_move(x, y);
    live.drain();
    let (px, py, under) = live.x.pointer();
    assert_eq!(
        (f64::from(px), f64::from(py)),
        (x, y),
        "the X server has the pointer elsewhere"
    );
    under
}

/// What an X source does as its drag starts: it finds the window under the
/// pointer -- the proxy, which the window manager maps over the whole
/// screen for the drag -- and announces the drag there, types then
/// position. That is how the window manager learns what is dragged; until
/// then it offers the drag to nothing (`DnDGrab` waits for the types).
/// Answers the proxy.
fn announce_to_proxy(live: &mut Live, owner: XWindow) -> XWindow {
    let (x, y, proxy) = live.x.pointer();
    assert_eq!(
        live.x.name_of(proxy),
        PROXY_NAME,
        "an X drag starts over the window manager's proxy"
    );
    let utf8 = live.x.atom("UTF8_STRING");
    let copy = live.x.atom("XdndActionCopy");
    live.x
        .xdnd_send(proxy, "XdndEnter", [owner, XDND_VERSION << 24, utf8, 0, 0]);
    live.x.xdnd_send(
        proxy,
        "XdndPosition",
        [owner, 0, packed(x, y), x11rb::CURRENT_TIME, copy],
    );
    live.drain();
    proxy
}

/// Presses on `rect`, has `live.x` take the `XdndSelection` -- the drag
/// starting -- and announces it to the proxy. Answers the drag's owner
/// window and the proxy.
fn start_x_drag(live: &mut Live, rect: Rect) -> (XWindow, XWindow) {
    press_at(&mut live.fixture.state, rect);
    live.drain();
    let owner = live.x.take_selection("XdndSelection");
    live.drain();
    assert!(taken_over(&live.fixture.state), "the X drag did not start");
    let proxy = announce_to_proxy(live, owner);
    (owner, proxy)
}

/// An X window of `client`, mapped, placed and a drop target.
fn x_target(live: &mut Live, client: &XClient, pixel: u32) -> (XWindow, Placement) {
    let xid = client.map(&Props::new(pixel));
    let id = live.managed(xid);
    client.xdnd_aware(xid);
    (xid, live.placement(id))
}

/// X to X: a drag from one X client's window dropped on another's. The
/// source finds the target itself under the pointer -- with the plain
/// focus it found the window manager's proxy, which relays only to
/// Wayland, and the drop went nowhere -- and the release ends scoot's side
/// of the drag without mapping the proxy back over the target.
#[test]
fn an_x_drag_drops_onto_another_x_clients_window() {
    let Some(mut live) = live("an_x_drag_drops_onto_another_x_clients_window") else {
        return;
    };
    let source = live.x.map(&Props::new(RED));
    let source = live.managed(source);
    let from = live.placement(source);
    let other = XClient::connect(live.display);
    let (target_xid, to) = x_target(&mut live, &other, BLUE);
    visible("drag source", &from);
    visible("drop target", &to);

    let (owner, proxy) = start_x_drag(&mut live, from.rect);

    let (x, y) = centre(to.rect);
    let under = move_and_look(&mut live, (x, y));
    assert_eq!(
        under,
        target_xid,
        "the X drag found {:?} under the pointer, not the X window there",
        live.x.name_of(under)
    );

    // What the source sends the window it found -- leaving the proxy for it
    // -- reaches the target...
    let utf8 = live.x.atom("UTF8_STRING");
    let copy = live.x.atom("XdndActionCopy");
    #[allow(clippy::cast_possible_truncation)]
    let at = packed(x as i16, y as i16);
    live.x.xdnd_send(proxy, "XdndLeave", [owner, 0, 0, 0, 0]);
    live.x
        .xdnd_send(under, "XdndEnter", [owner, XDND_VERSION << 24, utf8, 0, 0]);
    live.x.xdnd_send(
        under,
        "XdndPosition",
        [owner, 0, at, x11rb::CURRENT_TIME, copy],
    );
    let mut inbox = Inbox::new(&other);
    let enter = inbox.message(&mut live.fixture, "XdndEnter");
    assert_eq!(enter.data.as_data32()[0], owner);
    inbox.message(&mut live.fixture, "XdndPosition");

    // ...the release ends scoot's side of the drag...
    live.fixture
        .state
        .pointer_button(PointerButton::Left, false);
    settle(&mut live.fixture);
    assert!(
        !grabbed(&live.fixture.state),
        "the drag outlived the release"
    );
    assert_eq!(
        live.x.pointer().2,
        target_xid,
        "the release put the proxy back over the target"
    );

    // ...and the drop lands.
    live.x.xdnd_send(
        target_xid,
        "XdndDrop",
        [owner, 0, x11rb::CURRENT_TIME, 0, 0],
    );
    let dropped = inbox.message(&mut live.fixture, "XdndDrop");
    assert_eq!(dropped.data.as_data32()[0], owner);
}

/// Within one X window: moving selected text inside an editor is a drag
/// from a window onto itself, and needs the window itself under the
/// pointer too.
#[test]
fn an_x_drag_finds_its_own_window_under_the_pointer() {
    let Some(mut live) = live("an_x_drag_finds_its_own_window_under_the_pointer") else {
        return;
    };
    let xid = live.x.map(&Props::new(RED));
    let id = live.managed(xid);
    live.x.xdnd_aware(xid);
    let placement = live.placement(id);
    visible("window", &placement);
    start_x_drag(&mut live, placement.rect);
    let (x, y) = centre(placement.rect);
    let under = move_and_look(&mut live, (x + 10.0, y + 10.0));
    assert_eq!(
        under,
        xid,
        "the X drag found {:?} under the pointer, not its own window",
        live.x.name_of(under)
    );
    live.fixture
        .state
        .pointer_button(PointerButton::Left, false);
    live.drain();
    assert!(
        !grabbed(&live.fixture.state),
        "the drag outlived the release"
    );
}

/// The hovered X window closing mid-drag must not strand the proxy
/// unmapped: leaving it -- onto bare desktop -- still hands the drag back
/// to the proxy. (What the X variant carries the `X11Surface` for, rather
/// than looking the window up by surface as the drag moves: a closed window
/// is not there to look up.) The target is an override-redirect window --
/// a menu or tooltip, which the hit test names as the X focus too -- so its
/// closing moves no window of the layout under the drag.
#[test]
fn an_x_drag_whose_hovered_window_closes_finds_the_proxy_again() {
    let Some(mut live) = live("an_x_drag_whose_hovered_window_closes_finds_the_proxy_again") else {
        return;
    };
    let source = live.x.map(&Props::new(RED));
    let source = live.managed(source);
    let from = live.placement(source);
    visible("drag source", &from);
    let other = XClient::connect(live.display);
    let (cx, cy) = centre(from.rect);
    #[allow(clippy::cast_possible_truncation)]
    let menu = other.map(&Props {
        rect: (cx as i16 - 20, cy as i16 + 40, 40, 40),
        override_redirect: true,
        ..Props::new(BLUE)
    });
    other.xdnd_aware(menu);
    live.drain();
    start_x_drag(&mut live, from.rect);
    let under = move_and_look(&mut live, (cx, cy + 60.0));
    assert_eq!(under, menu, "over the menu: {:?}", live.x.name_of(under));

    use x11rb::connection::Connection as _;
    use x11rb::protocol::xproto::ConnectionExt as _;
    other
        .conn
        .destroy_window(menu)
        .expect("a destroy request")
        .check()
        .expect("the X server destroyed the menu");
    other.conn.flush().expect("flushed");
    live.drain();

    // Bare desktop: above the window, in the layout's top gap.
    assert!(from.rect.y > 4, "no gap above the window: {from:?}");
    let under = move_and_look(&mut live, (cx, 2.0));
    assert_eq!(
        live.x.name_of(under),
        PROXY_NAME,
        "after the hovered X window closed, the X drag lost the proxy"
    );
    live.fixture
        .state
        .pointer_button(PointerButton::Left, false);
    live.drain();
    assert!(
        !grabbed(&live.fixture.state),
        "the drag outlived the release"
    );
}

/// The Wayland peer's drag, started from its window on the press the test
/// just made there.
fn start_wayland_drag(live: &mut Live, from: Rect, payload: &Arc<Vec<u8>>) {
    press_at(&mut live.fixture.state, from);
    live.drain();
    let serial = live
        .fixture
        .state
        .seat
        .get_pointer()
        .and_then(|pointer| pointer.with_grab(|serial, _| serial))
        .expect("the press holds the pointer");
    assert!(matches!(
        live.fixture.run(Step::Clip(ClipStep::Drag {
            mime: MIME,
            payload: payload.clone(),
            serial: serial.into(),
        })),
        Ack::Done
    ));
    live.drain();
    assert!(
        taken_over(&live.fixture.state),
        "the Wayland drag did not start"
    );
}

/// Wayland to X: the drop lands, bytes and all. Over the X window the
/// window manager speaks XDND to it on the Wayland source's behalf; the
/// target accepts, the release drops, and converting `XdndSelection` reads
/// the Wayland source's payload.
#[test]
fn a_wayland_drag_drops_onto_an_x_window() {
    let Some(mut live) = live("a_wayland_drag_drops_onto_an_x_window") else {
        return;
    };
    let wayland = live.map_peer("wayland");
    assert!(matches!(
        live.fixture.run(Step::Clip(ClipStep::Bind)),
        Ack::Done
    ));
    let target_client = XClient::connect(live.display);
    let (target_xid, to) = x_target(&mut live, &target_client, RED);
    let from = live.placement(wayland);
    visible("Wayland window", &from);
    visible("drop target", &to);
    let payload = Arc::new(b"dropped from Wayland".to_vec());
    start_wayland_drag(&mut live, from.rect, &payload);

    let (x, y) = centre(to.rect);
    live.fixture.state.pointer_move(x, y);
    settle(&mut live.fixture);
    let utf8 = target_client.atom("UTF8_STRING");
    let copy = target_client.atom("XdndActionCopy");
    let mut inbox = Inbox::new(&target_client);
    let enter = inbox.message(&mut live.fixture, "XdndEnter");
    let enter = enter.data.as_data32();
    let source = enter[0];
    assert!(enter[1] >> 24 >= 2, "an XDND version below 2: {enter:?}");
    assert!(
        enter[2..].contains(&utf8),
        "the Wayland type was not offered: {enter:?}"
    );
    let position = inbox.message(&mut live.fixture, "XdndPosition");
    #[allow(clippy::cast_possible_truncation)]
    let at = packed(x as i16, y as i16);
    assert_eq!(
        position.data.as_data32()[2],
        at,
        "XdndPosition is not where the pointer is"
    );
    target_client.xdnd_send(source, "XdndStatus", [target_xid, 1, 0, 0, copy]);
    settle(&mut live.fixture);

    live.fixture
        .state
        .pointer_button(PointerButton::Left, false);
    settle(&mut live.fixture);
    inbox.message(&mut live.fixture, "XdndDrop");
    target_client.convert("XdndSelection", utf8, "SCOOT_DROP", target_xid);
    let notify = inbox.selection_notify(&mut live.fixture);
    assert_ne!(notify.property, x11rb::NONE, "the conversion was refused");
    assert_eq!(
        target_client.read_property(target_xid, "SCOOT_DROP"),
        *payload,
        "the drop did not carry the Wayland source's bytes"
    );
    target_client.xdnd_send(source, "XdndFinished", [target_xid, 1, copy, 0, 0]);
    settle(&mut live.fixture);
    assert!(
        !grabbed(&live.fixture.state),
        "the drag outlived the release"
    );
}

/// A Wayland drag over an X window that takes no drops (no `XdndAware`):
/// no XDND reaches it, and the release ends the drag cleanly.
#[test]
fn a_wayland_drag_over_an_x_window_that_takes_no_drops_ends_cleanly() {
    let Some(mut live) = live("a_wayland_drag_over_an_x_window_that_takes_no_drops_ends_cleanly")
    else {
        return;
    };
    let wayland = live.map_peer("wayland");
    assert!(matches!(
        live.fixture.run(Step::Clip(ClipStep::Bind)),
        Ack::Done
    ));
    let target_client = XClient::connect(live.display);
    let xid = target_client.map(&Props::new(RED));
    let id = live.managed(xid);
    let (from, to) = (live.placement(wayland), live.placement(id));
    visible("Wayland window", &from);
    visible("X window", &to);
    let payload = Arc::new(b"nowhere to go".to_vec());
    start_wayland_drag(&mut live, from.rect, &payload);
    live.fixture
        .state
        .pointer_move(centre(to.rect).0, centre(to.rect).1);
    settle(&mut live.fixture);
    live.fixture
        .state
        .pointer_button(PointerButton::Left, false);
    settle(&mut live.fixture);
    let mut inbox = Inbox::new(&target_client);
    assert!(
        !inbox.has_message("XdndEnter") && !inbox.has_message("XdndDrop"),
        "XDND reached a window that never said it takes drops"
    );
    assert!(
        !grabbed(&live.fixture.state),
        "the drag outlived the release"
    );
    // And the session still takes input: a click on the X window focuses it.
    live.fixture
        .state
        .pointer_move(centre(to.rect).0, centre(to.rect).1);
    live.fixture.state.pointer_button(PointerButton::Left, true);
    live.fixture
        .state
        .pointer_button(PointerButton::Left, false);
    live.drain();
    assert_eq!(live.fixture.state.focus, Some(id));
}

/// Motion within one X window is one `enter`, not one per motion: the X
/// focus compares as the same focus from motion to motion (a surface and a
/// window id -- see `PointerFocus`'s `PartialEq`), so Smithay delivers
/// plain motion rather than a leave and a fresh enter each time. Pinned on
/// scoot's side, where it bites: every `enter` files a serial in the
/// interaction ring (`record_pointer_enter`), sixteen entries shared with
/// the key and button presses the activation and drag gates look for --
/// an enter per motion would flush those out within sixteen motions.
/// (XWayland itself turns a same-surface leave and enter into no X
/// crossing at all, measured, so the X side cannot see the difference;
/// it is checked here only as a sanity check.)
#[test]
fn motion_within_an_x_window_enters_it_once() {
    let Some(mut live) = live("motion_within_an_x_window_enters_it_once") else {
        return;
    };
    let xid = live.x.map(&Props::new(RED));
    let id = live.managed(xid);
    live.x.select_crossings(xid);
    let placement = live.placement(id);
    visible("X window", &placement);
    let (x, y) = centre(placement.rect);
    live.fixture.state.pointer_move(x, y);
    let entered = live.fixture.state.interaction_serials.latest();
    assert!(entered.is_some(), "moving onto the X window filed no enter");
    for step in 1..=20 {
        live.fixture.state.pointer_move(x + f64::from(step), y);
    }
    assert_eq!(
        live.fixture.state.interaction_serials.latest(),
        entered,
        "motion within one X window filed more enters"
    );
    live.drain();
    let mut inbox = Inbox::new(&live.x);
    assert_eq!(
        inbox.crossings(),
        (1, 0),
        "(enters, leaves) for twenty-one motions inside one X window"
    );
}

/// X to Wayland keeps working after an X drag has crossed an X window --
/// every one does, starting over its own: over the X window the proxy is
/// out of the way (the source finds its own window), and over a Wayland
/// window it is back under the pointer, which is what an X-to-Wayland drop
/// drops on. Pins the fork's `6e6fe896`: without that flush of the remap the
/// X server still had the proxy unmapped over the Wayland window and the
/// source found "no window" there (3 of 3 runs at `74edbf32`).
#[test]
fn an_x_drag_crossing_an_x_window_still_finds_the_proxy_over_wayland() {
    let Some(mut live) = live("an_x_drag_crossing_an_x_window_still_finds_the_proxy_over_wayland")
    else {
        return;
    };
    let wayland = live.map_peer("wayland");
    let xid = live.x.map(&Props::new(RED));
    let id = live.managed(xid);
    let (on_x, on_wayland) = (live.placement(id), live.placement(wayland));
    visible("X window", &on_x);
    visible("Wayland window", &on_wayland);
    start_x_drag(&mut live, on_x.rect);
    let (x, y) = centre(on_x.rect);
    let under = move_and_look(&mut live, (x + 5.0, y + 5.0));
    assert_eq!(
        under,
        xid,
        "over its own window: {:?}",
        live.x.name_of(under)
    );
    let under = move_and_look(&mut live, centre(on_wayland.rect));
    assert_eq!(
        live.x.name_of(under),
        PROXY_NAME,
        "over a Wayland window the X drag must find the proxy"
    );
    live.fixture
        .state
        .pointer_button(PointerButton::Left, false);
    live.drain();
    assert!(
        !grabbed(&live.fixture.state),
        "the drag outlived the release"
    );
}

/// A drag from Wayland over an X window that closes under it: leaving the
/// closed window still clears the window manager's XDND offer, so the next
/// X drag starts. (What the X variant carries the `X11Surface` for, rather
/// than looking the window up by surface as the drag moves: a closed window
/// is not there to look up, and an offer left behind makes the window
/// manager take `XdndSelection` back from every later X drag.) The window is
/// an override-redirect one -- a menu or tooltip, which the hit test names
/// as the X focus too -- so its closing moves nothing in the layout.
#[test]
fn a_wayland_drag_whose_x_window_closes_leaves_x_drags_working() {
    let Some(mut live) = live("a_wayland_drag_whose_x_window_closes_leaves_x_drags_working") else {
        return;
    };
    let wayland = live.map_peer("wayland");
    assert!(matches!(
        live.fixture.run(Step::Clip(ClipStep::Bind)),
        Ack::Done
    ));
    let xid = live.x.map(&Props::new(RED));
    let id = live.managed(xid);
    let (from, on_x) = (live.placement(wayland), live.placement(id));
    visible("Wayland window", &from);
    visible("X window", &on_x);
    let other = XClient::connect(live.display);
    let (cx, cy) = centre(from.rect);
    #[allow(clippy::cast_possible_truncation)]
    let menu = other.map(&Props {
        rect: (cx as i16 - 20, cy as i16 + 40, 40, 40),
        override_redirect: true,
        ..Props::new(BLUE)
    });
    other.xdnd_aware(menu);
    live.drain();
    let payload = Arc::new(b"into a closing menu".to_vec());
    start_wayland_drag(&mut live, from.rect, &payload);
    live.fixture.state.pointer_move(cx, cy + 60.0);
    settle(&mut live.fixture);
    let mut inbox = Inbox::new(&other);
    inbox.message(&mut live.fixture, "XdndEnter");

    use x11rb::connection::Connection as _;
    use x11rb::protocol::xproto::ConnectionExt as _;
    other
        .conn
        .destroy_window(menu)
        .expect("a destroy request")
        .check()
        .expect("the X server destroyed the menu");
    other.conn.flush().expect("flushed");
    settle(&mut live.fixture);
    assert!(from.rect.y > 4, "no gap above the window: {from:?}");
    live.fixture.state.pointer_move(cx, 2.0);
    settle(&mut live.fixture);
    live.fixture
        .state
        .pointer_button(PointerButton::Left, false);
    settle(&mut live.fixture);
    assert!(
        !grabbed(&live.fixture.state),
        "the drag outlived the release"
    );

    press_at(&mut live.fixture.state, on_x.rect);
    live.drain();
    live.x.take_selection("XdndSelection");
    live.drain();
    assert!(
        taken_over(&live.fixture.state),
        "an X drag could not start after a Wayland drag lost its X window"
    );
    live.fixture
        .state
        .pointer_button(PointerButton::Left, false);
    live.drain();
}
