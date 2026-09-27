//! How a drop from Wayland onto an X window ends, whatever the X target
//! does after it -- and that no ending leaves X drags unable to start.
//!
//! The window manager keeps the drag's X offer (`X11Wm`'s `active_offer`)
//! until the target finishes, and while it has one it takes
//! `XdndSelection` back from any X client that takes it: that is how it
//! keeps a rough X client from hijacking a Wayland drag in flight. So an
//! offer left behind -- a target that never answered, or that died or hung
//! before `XdndFinished` -- stopped every later X drag, X to Wayland
//! included, until scoot restarted. Each case here drops onto a second X
//! client's window, lets it end one way, then starts an X drag from the
//! first X client's window.
//!
//! These pin scoot-sh/smithay `7388af13` (the source told once) and
//! `9515d7e5` (the offer ends with its target, or when a new X drag starts
//! after the drop); all six failed at `6e6fe896`, and with `7388af13` alone
//! the dying- and hanging-after-the-drop cases still did.
//!
//! The Wayland source is told how the drop went exactly once:
//! `dnd_drop_performed` once for a drop made, then `dnd_finished` or
//! `cancelled`; `cancelled` alone for one the target never accepted, which
//! is not dropped on it at all (`XdndLeave`, not `XdndDrop`).

use std::sync::Arc;

use scoot_ipc::PointerButton;
use x11rb::protocol::xproto::Window as XWindow;

use super::dnd::{press_at, taken_over};
use super::drop::{centre, grabbed, settle, start_wayland_drag, visible};
use super::live::{BLUE, Live, RED, live};
use super::peer::{Ack, ClipStep, Step};
use super::x11::{Props, XClient};
use super::xdnd::Inbox;

/// What the X target does once the Wayland drag is over it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    /// Answers nothing, not even `XdndStatus`: a hung or broken target.
    Silent,
    /// Answers `XdndStatus` refusing the drop.
    Refuses,
    /// Answers `XdndStatus` accepting the drop.
    Accepts,
}

/// A Wayland window, an X window of `live.x` to start an X drag from
/// later, and a drop target of a second X client; a Wayland drag moved
/// over the target, which answers as `answer` says. Answers the target's
/// client, its window, the XWM's selection window the drag speaks from,
/// and the rect to start the later X drag on.
///
/// The target is an override-redirect window (a menu, say) over the
/// Wayland window, which the hit test names as an X focus like any other:
/// two columns fit on screen, three would not.
fn drag_over_target(
    live: &mut Live,
    answer: Target,
) -> (XClient, XWindow, XWindow, scoot_core::Rect) {
    let wayland = live.map_peer("wayland");
    assert!(matches!(
        live.fixture.run(Step::Clip(ClipStep::Bind)),
        Ack::Done
    ));
    let x_source = live.x.map(&Props::new(BLUE));
    let x_source = live.managed(x_source);
    let (from, x_from) = (live.placement(wayland), live.placement(x_source));
    visible("Wayland window", &from);
    visible("X drag source", &x_from);
    let target_client = XClient::connect(live.display);
    let (cx, cy) = centre(from.rect);
    #[allow(clippy::cast_possible_truncation)]
    let target_xid = target_client.map(&Props {
        rect: (cx as i16 - 20, cy as i16 + 40, 40, 40),
        override_redirect: true,
        ..Props::new(RED)
    });
    target_client.xdnd_aware(target_xid);
    live.drain();
    let payload = Arc::new(b"dropped from Wayland".to_vec());
    start_wayland_drag(live, from.rect, &payload);

    live.fixture.state.pointer_move(cx, cy + 60.0);
    settle(&mut live.fixture);
    let mut inbox = Inbox::new(&target_client);
    let source = inbox
        .message(&mut live.fixture, "XdndEnter")
        .data
        .as_data32()[0];
    inbox.message(&mut live.fixture, "XdndPosition");
    let copy = target_client.atom("XdndActionCopy");
    match answer {
        Target::Silent => {}
        Target::Refuses => {
            target_client.xdnd_send(source, "XdndStatus", [target_xid, 0, 0, 0, 0]);
        }
        Target::Accepts => {
            target_client.xdnd_send(source, "XdndStatus", [target_xid, 1, 0, 0, copy]);
        }
    }
    settle(&mut live.fixture);
    drop(inbox);
    (target_client, target_xid, source, x_from.rect)
}

/// Releases the button: the drop. Answers whether the target was sent
/// `XdndDrop` (else it must have been sent `XdndLeave`).
fn release(live: &mut Live, target_client: &XClient) -> bool {
    live.fixture
        .state
        .pointer_button(PointerButton::Left, false);
    settle(&mut live.fixture);
    assert!(
        !grabbed(&live.fixture.state),
        "the drag outlived the release"
    );
    let mut inbox = Inbox::new(target_client);
    if inbox.has_message("XdndDrop") {
        return true;
    }
    inbox.message(&mut live.fixture, "XdndLeave");
    false
}

/// What the Wayland source has been told of its drag.
fn source_events(live: &mut Live) -> Vec<&'static str> {
    let Ack::Events(events) = live.fixture.run(Step::Clip(ClipStep::DragEvents)) else {
        panic!("the peer did not list its drag events");
    };
    events
}

/// Starts an X drag from `live.x`'s window at `rect` and asserts the
/// window manager took it; releases it.
fn x_drag_starts(live: &mut Live, rect: scoot_core::Rect, after: &str) {
    press_at(&mut live.fixture.state, rect);
    live.drain();
    live.x.take_selection("XdndSelection");
    live.drain();
    assert!(
        taken_over(&live.fixture.state),
        "an X drag could not start after {after}"
    );
    live.fixture
        .state
        .pointer_button(PointerButton::Left, false);
    live.drain();
}

/// The control: a target that accepts, reads and finishes. The source hears
/// `dnd_drop_performed` once, then `dnd_finished`.
#[test]
fn an_x_target_that_finishes_leaves_x_drags_working() {
    let Some(mut live) = live("an_x_target_that_finishes_leaves_x_drags_working") else {
        return;
    };
    let (target_client, target_xid, source, x_from) = drag_over_target(&mut live, Target::Accepts);
    assert!(release(&mut live, &target_client), "no XdndDrop");
    let copy = target_client.atom("XdndActionCopy");
    target_client.xdnd_send(source, "XdndFinished", [target_xid, 1, copy, 0, 0]);
    settle(&mut live.fixture);
    assert_eq!(
        source_events(&mut live),
        ["dnd_drop_performed", "dnd_finished"]
    );
    x_drag_starts(&mut live, x_from, "a finished drop");
}

/// A target that refuses the drop is not dropped on: it gets `XdndLeave`,
/// and the source hears `cancelled` alone -- not `dnd_drop_performed` for a
/// drop that was never made.
#[test]
fn an_x_target_that_refuses_is_left_not_dropped_on() {
    let Some(mut live) = live("an_x_target_that_refuses_is_left_not_dropped_on") else {
        return;
    };
    let (target_client, _, _, x_from) = drag_over_target(&mut live, Target::Refuses);
    assert!(
        !release(&mut live, &target_client),
        "a target that refused the drop was sent XdndDrop"
    );
    assert_eq!(source_events(&mut live), ["cancelled"]);
    x_drag_starts(&mut live, x_from, "a refused drop");
}

/// A target that never answers `XdndStatus`, still alive: the release is no
/// drop, and nothing waits on the target.
#[test]
fn an_x_target_that_never_answers_leaves_x_drags_working() {
    let Some(mut live) = live("an_x_target_that_never_answers_leaves_x_drags_working") else {
        return;
    };
    let (target_client, _, _, x_from) = drag_over_target(&mut live, Target::Silent);
    assert!(
        !release(&mut live, &target_client),
        "a target that never answered was sent XdndDrop"
    );
    assert_eq!(source_events(&mut live), ["cancelled"]);
    x_drag_starts(&mut live, x_from, "a drop onto a silent target");
    drop(target_client);
}

/// The same target, then gone: its client disconnects.
#[test]
fn an_x_target_that_never_answers_and_dies_leaves_x_drags_working() {
    let Some(mut live) = live("an_x_target_that_never_answers_and_dies_leaves_x_drags_working")
    else {
        return;
    };
    let (target_client, _, _, x_from) = drag_over_target(&mut live, Target::Silent);
    live.fixture
        .state
        .pointer_button(PointerButton::Left, false);
    settle(&mut live.fixture);
    drop(target_client);
    live.drain();
    x_drag_starts(&mut live, x_from, "a silent target died");
}

/// A target that accepts, is dropped on, and dies before `XdndFinished`:
/// its windows' destruction ends the drop, and the source hears
/// `cancelled` after `dnd_drop_performed`.
#[test]
fn an_x_target_dying_after_the_drop_leaves_x_drags_working() {
    let Some(mut live) = live("an_x_target_dying_after_the_drop_leaves_x_drags_working") else {
        return;
    };
    let (target_client, _, _, x_from) = drag_over_target(&mut live, Target::Accepts);
    assert!(release(&mut live, &target_client), "no XdndDrop");
    drop(target_client);
    live.drain();
    assert_eq!(
        source_events(&mut live),
        ["dnd_drop_performed", "cancelled"]
    );
    x_drag_starts(&mut live, x_from, "the target died after the drop");
}

/// A target that accepts, is dropped on, and then hangs -- alive, never
/// finishing. The next X drag starts all the same: the drop is abandoned
/// (the source hears `cancelled`) rather than holding `XdndSelection` for a
/// target that may never answer.
#[test]
fn an_x_target_hanging_after_the_drop_leaves_x_drags_working() {
    let Some(mut live) = live("an_x_target_hanging_after_the_drop_leaves_x_drags_working") else {
        return;
    };
    let (target_client, _, _, x_from) = drag_over_target(&mut live, Target::Accepts);
    assert!(release(&mut live, &target_client), "no XdndDrop");
    x_drag_starts(&mut live, x_from, "the target hung after the drop");
    assert_eq!(
        source_events(&mut live),
        ["dnd_drop_performed", "cancelled"]
    );
    drop(target_client);
}
