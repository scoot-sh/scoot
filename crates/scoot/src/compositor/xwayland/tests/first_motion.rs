//! An X drag's first motion onto another X window, and a drag crossing
//! from one X window onto another. See
//! `docs/backlog/resolved/xwayland-x-drag-first-motion-race-done.md`.
//!
//! GTK and Qt start an X drag by taking `XdndSelection` and at once looking
//! for the window under the pointer -- before the window manager has made
//! its full-screen proxy, so they find their own window and name their
//! types to nobody. Smithay's drag grab enters no target until the types
//! are known, so on the drag's first motion onto another X window the proxy
//! was still over it: the source found the proxy, and a release before the
//! next motion dropped nothing (live, GTK `mousepad`: 20 of 20). And a drag
//! crossing between two X windows maps the proxy back as it leaves the
//! first and unmaps it as it enters the second, so a source that looks in
//! between finds the proxy over the second (live: 2-3 of 20).
//!
//! Both are fixed in scoot-sh/smithay `b1ac3ca7` (`DndFocus::
//! enter_needs_metadata`, which `pointer_focus.rs` forwards). These pins
//! fail at `d3a4cd73`, the fork rev before it (the first with "found
//! \"Smithay XDND proxy\"", the other two with "mapped the proxy back in
//! between"), and pass at `b1ac3ca7`.
//!
//! `an_x_drag_over_a_window_that_remaps_keeps_the_proxy_away` pins the
//! review fix on top, `7e18b661`: it fails at `b1ac3ca7`.
//!
//! `first_motion_race.rs` measures the timing half: that the window
//! manager's unmap and the motion reach the source unordered.

use scoot_ipc::PointerButton;
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::xproto::{ChangeWindowAttributesAux, ConnectionExt as _, EventMask};

use super::dnd::{press_at, taken_over};
use super::drop::{centre, grabbed, move_and_look, settle, start_x_drag, visible, x_target};
use super::live::{BLUE, RED, live};
use super::x11::{Props, XClient};
use super::xdnd::{Inbox, XDND_VERSION, packed};

/// The drag starts with no word to the proxy, as a toolkit's does; its one
/// motion lands on another client's X window; the source finds that window
/// there, not the proxy, and the drop released without another motion
/// lands.
#[test]
fn an_x_drags_first_motion_onto_an_x_window_finds_that_window() {
    let Some(mut live) = live("an_x_drags_first_motion_onto_an_x_window_finds_that_window") else {
        return;
    };
    let source = live.x.map(&Props::new(RED));
    let source = live.managed(source);
    let from = live.placement(source);
    let other = XClient::connect(live.display);
    let (target_xid, to) = x_target(&mut live, &other, BLUE);
    visible("drag source", &from);
    visible("drop target", &to);

    press_at(&mut live.fixture.state, from.rect);
    live.drain();
    let owner = live.x.take_selection("XdndSelection");
    live.drain();
    assert!(taken_over(&live.fixture.state), "the X drag did not start");

    let (x, y) = centre(to.rect);
    let under = move_and_look(&mut live, (x, y));
    assert_eq!(
        under,
        target_xid,
        "on the X drag's first motion the source found {:?} under the pointer, not the X \
         window there",
        live.x.name_of(under)
    );

    // What the source then does -- speaks XDND to the window it found, which
    // accepts -- and the release, with no motion in between.
    let utf8 = live.x.atom("UTF8_STRING");
    let copy = live.x.atom("XdndActionCopy");
    #[allow(clippy::cast_possible_truncation)]
    let at = packed(x as i16, y as i16);
    live.x
        .xdnd_send(under, "XdndEnter", [owner, XDND_VERSION << 24, utf8, 0, 0]);
    live.x.xdnd_send(
        under,
        "XdndPosition",
        [owner, 0, at, x11rb::CURRENT_TIME, copy],
    );
    let mut inbox = Inbox::new(&other);
    inbox.message(&mut live.fixture, "XdndPosition");
    other.xdnd_send(owner, "XdndStatus", [target_xid, 1, 0, 0, copy]);
    live.fixture
        .state
        .pointer_button(PointerButton::Left, false);
    settle(&mut live.fixture);
    assert!(
        !grabbed(&live.fixture.state),
        "the drag outlived the release"
    );
    live.x.xdnd_send(
        target_xid,
        "XdndDrop",
        [owner, 0, x11rb::CURRENT_TIME, 0, 0],
    );
    let dropped = inbox.message(&mut live.fixture, "XdndDrop");
    assert_eq!(dropped.data.as_data32()[0], owner);
}

/// A drag crossing from one X window straight onto another leaves the
/// proxy unmapped: leaving the first maps it back and entering the second
/// unmaps it, and in that order an X source could find the proxy over the
/// second window between the two -- GTK did, live, in 2-3 of 20 drags
/// released after one motion onto the other window, and those drops did
/// nothing. Pinned on what the source is told: no `MapNotify` for the
/// proxy on that motion. Onto another client's window...
#[test]
fn an_x_drag_crossing_onto_another_clients_window_never_maps_the_proxy() {
    crossing_never_maps_the_proxy(
        "an_x_drag_crossing_onto_another_clients_window_never_maps_the_proxy",
        false,
    );
}

/// ...and onto another window of the source's own client (two windows of
/// one app instance).
#[test]
fn an_x_drag_crossing_onto_its_own_clients_other_window_never_maps_the_proxy() {
    crossing_never_maps_the_proxy(
        "an_x_drag_crossing_onto_its_own_clients_other_window_never_maps_the_proxy",
        true,
    );
}

fn crossing_never_maps_the_proxy(test: &str, same_client: bool) {
    let Some(mut live) = live(test) else {
        return;
    };
    let source = live.x.map(&Props::new(RED));
    let source_id = live.managed(source);
    let from = live.placement(source_id);
    let other = XClient::connect(live.display);
    let (target_xid, to) = if same_client {
        let xid = live.x.map(&Props::new(BLUE));
        let id = live.managed(xid);
        live.x.xdnd_aware(xid);
        (xid, live.placement(id))
    } else {
        x_target(&mut live, &other, BLUE)
    };
    visible("drag source", &from);
    visible("drop target", &to);
    live.x
        .conn
        .change_window_attributes(
            live.x.root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY),
        )
        .expect("an attributes request")
        .check()
        .expect("the X server accepted the event mask");

    let (_, proxy) = start_x_drag(&mut live, from.rect);
    let (x, y) = centre(from.rect);
    let under = move_and_look(&mut live, (x + 5.0, y + 5.0));
    assert_eq!(
        under,
        source,
        "over its own window: {:?}",
        live.x.name_of(under)
    );
    live.x.drain();

    let under = move_and_look(&mut live, centre(to.rect));
    assert_eq!(
        under,
        target_xid,
        "over the other: {:?}",
        live.x.name_of(under)
    );
    let remapped = live
        .x
        .drain()
        .iter()
        .any(|event| matches!(event, XEvent::MapNotify(map) if map.window == proxy));
    assert!(
        !remapped,
        "crossing from one X window onto another mapped the proxy back in between"
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

/// The X window under an X drag is unmapped and mapped again between two
/// motions (a dialog reopening, say): the drag's next motion finds the same
/// window under a new `wl_surface`, a new focus, so the grab enters it
/// before leaving the old one. The proxy must stay out of the way: in that
/// order, leaving the old surface mapped the proxy back over the window the
/// drag had just entered (review of `b1ac3ca7`), and the source found the
/// proxy there until the drag left the window.
///
/// A managed window: an unmanaged one re-derives the pointer focus as it
/// unmaps (`x11_unmanaged` removal), so the grab has left it by the time it
/// maps again and the order never arises.
#[test]
fn an_x_drag_over_a_window_that_remaps_keeps_the_proxy_away() {
    let Some(mut live) = live("an_x_drag_over_a_window_that_remaps_keeps_the_proxy_away") else {
        return;
    };
    let source = live.x.map(&Props::new(RED));
    let source = live.managed(source);
    let from = live.placement(source);
    let other = XClient::connect(live.display);
    let (target_xid, to) = x_target(&mut live, &other, BLUE);
    visible("drag source", &from);
    visible("drop target", &to);

    start_x_drag(&mut live, from.rect);
    let over = centre(to.rect);
    let under = move_and_look(&mut live, over);
    assert_eq!(
        under,
        target_xid,
        "over the target: {:?}",
        live.x.name_of(under)
    );

    other.unmap(target_xid);
    other
        .conn
        .map_window(target_xid)
        .expect("a map request")
        .check()
        .expect("the X server accepted the map");
    let again = live.managed(target_xid);
    let to = live.placement(again);
    #[allow(clippy::cast_possible_truncation)]
    let still_under = to
        .rect
        .contains(scoot_core::Point::new(over.0 as i32 + 1, over.1 as i32));
    assert!(
        still_under,
        "the window mapped back elsewhere ({:?}), so this does not test a remap in place",
        to.rect
    );

    let under = move_and_look(&mut live, (over.0 + 1.0, over.1));
    assert_eq!(
        under,
        target_xid,
        "after the window mapped again the source found {:?} under the pointer, not the \
         window",
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
