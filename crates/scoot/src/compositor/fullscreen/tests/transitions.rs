//! What a client is told: the configure's size and `fullscreen` bit through
//! entering, leaving, the frame race on the way out, unmapping, locking, and
//! the requests that arrive by some other road (the bind, IPC).

use scoot_core::Action;
use scoot_ipc::{Request, Response};

use super::*;

#[test]
fn set_fullscreen_configures_the_output_size_with_the_state_bit() {
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    let tiled = fixture.rect_of(0);

    let entered = fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    assert!(entered.fullscreen, "{entered:?}");
    assert_eq!((entered.width, entered.height), (CANVAS, CANVAS));
    assert!(fixture.state.world.is_fullscreen(fixture.id(0)));
    assert_eq!(fixture.rect_of(0), OUTPUT);

    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    let left = fixture.configured(Step::UnsetFullscreen { window: 0 });
    assert!(!left.fullscreen, "{left:?}");
    assert_eq!(
        (left.width, left.height),
        (tiled.w, tiled.h),
        "leaving must hand back the size it had before"
    );
    assert_eq!(fixture.rect_of(0), tiled);
}

#[test]
fn every_request_is_answered_with_a_configure_even_when_nothing_changes() {
    // xdg-shell: "the compositor will respond by emitting a configure event"
    // -- for both requests, whatever the answer.
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    let before = fixture.configures(0).len();
    fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    let first = fixture.configures(0).len();
    assert!(first > before);
    let again = fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    assert!(again.fullscreen);
    assert_eq!(
        fixture.configures(0).len(),
        first + 1,
        "a repeat went unanswered"
    );

    fixture.configured(Step::UnsetFullscreen { window: 0 });
    let left = fixture.configures(0).len();
    let still = fixture.configured(Step::UnsetFullscreen { window: 0 });
    assert!(!still.fullscreen);
    assert_eq!(fixture.configures(0).len(), left + 1);
}

#[test]
fn a_request_before_the_first_commit_is_what_the_first_configure_carries() {
    // `foot --fullscreen`: set_fullscreen goes out before the initial commit, so the
    // window's very first frame is already the fullscreen one.
    let mut fixture = Fixture::new();
    fixture.map_with(WINDOW_BGRA, true);
    let configures = fixture.configures(0);
    let drawn_for = configures
        .iter()
        .rev()
        .find(|c| c.width > 0)
        .copied()
        .expect("a sized configure");
    assert!(drawn_for.fullscreen, "{configures:?}");
    assert_eq!((drawn_for.width, drawn_for.height), (CANVAS, CANVAS));
    assert_eq!(
        fixture
            .state
            .world
            .fullscreen_on(fixture.state.outputs.primary_id().unwrap()),
        Some(fixture.id(0))
    );
}

#[test]
fn a_late_fullscreen_frame_after_leaving_does_not_widen_the_column() {
    // The race a video player hits on the way out: the tiled configure goes
    // out while its next frame is still sized for the whole output. That
    // frame answers the *fullscreen* configure (it has not acked the new
    // one), so it must not read as a window refusing to shrink.
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.map(OTHER_BGRA);
    let tiled = fixture.rect_of(1);

    fixture.configured(Step::SetFullscreen {
        window: 1,
        output: None,
    });
    fixture.done(Step::Draw {
        window: 1,
        color: OTHER_BGRA,
    });
    fixture.configured(Step::UnsetFullscreen { window: 1 });
    fixture.done(Step::DrawUnacked {
        window: 1,
        width: CANVAS,
        height: CANVAS,
    });
    // ...and then it catches up.
    fixture.done(Step::Draw {
        window: 1,
        color: OTHER_BGRA,
    });
    assert_eq!(fixture.rect_of(1), tiled, "the column did not come back");
    // Nothing learned either: cycling width still lands on the presets.
    let configures = fixture.configures(1);
    let last = configures.last().unwrap();
    assert_eq!((last.width, last.height), (tiled.w, tiled.h));
}

#[test]
fn unmapping_discards_fullscreen_and_the_remap_is_not_fullscreen() {
    // xdg-shell: an unmapped toplevel "returns to the state it had right
    // after xdg_surface.get_toplevel" -- which was not fullscreen.
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    assert!(fixture.state.world.is_fullscreen(fixture.id(0)));

    fixture.done(Step::Unmap { window: 0 });
    assert!(
        !fixture.state.world.is_fullscreen(fixture.id(0)),
        "fullscreen survived an unmap"
    );
    let used = fixture.configured(Step::Remap {
        window: 0,
        color: WINDOW_BGRA,
    });
    assert!(!used.fullscreen, "{used:?}");
    assert!(!fixture.state.world.is_fullscreen(fixture.id(0)));
}

#[test]
fn an_unmap_of_a_window_that_is_not_fullscreen_changes_nothing_here() {
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.map(OTHER_BGRA);
    let before = fixture.state.world.arrange();
    fixture.done(Step::Unmap { window: 0 });
    assert_eq!(fixture.state.world.arrange(), before);
}

#[test]
fn the_bind_and_ipc_toggle_the_focused_window() {
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    let id = fixture.id(0);

    // Super+f, pressed the way a user presses it.
    fixture
        .state
        .press(&scoot_ipc::KeyCombo {
            key: "f".into(),
            modifiers: vec![scoot_ipc::Modifier::Super],
        })
        .expect("a pressable combo");
    fixture.settle();
    assert!(fixture.state.world.is_fullscreen(id));
    let last = *fixture.configures(0).last().unwrap();
    assert!(last.fullscreen && last.width == CANVAS, "{last:?}");

    let response = fixture
        .state
        .handle_request(Request::Action(scoot_ipc::Action::ToggleFullscreen));
    assert!(matches!(response, Response::Ok { .. }), "{response:?}");
    fixture.settle();
    assert!(!fixture.state.world.is_fullscreen(id));
    let last = *fixture.configures(0).last().unwrap();
    assert!(!last.fullscreen, "{last:?}");
}

#[test]
fn ipc_set_fullscreen_targets_a_window_and_tells_it_even_while_invisible() {
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.map(OTHER_BGRA);
    // Put the first window on another workspace, out of sight.
    fixture.state.act(Action::FocusWindowId(fixture.id(0)));
    fixture.state.act(Action::MoveWindowToWorkspaceIndex(1));
    fixture.state.act(Action::FocusWindowId(fixture.id(1)));
    fixture.settle();
    let hidden = fixture.state.world.arrange();
    assert!(!hidden.get(fixture.id(0)).unwrap().visible);

    let response =
        fixture
            .state
            .handle_request(Request::Action(scoot_ipc::Action::SetFullscreen {
                id: fixture.id(0).0,
                fullscreen: true,
            }));
    assert!(matches!(response, Response::Ok { .. }), "{response:?}");
    fixture.settle();
    assert!(fixture.state.world.is_fullscreen(fixture.id(0)));
    let last = *fixture.configures(0).last().unwrap();
    assert!(
        last.fullscreen,
        "an invisible window was not told: {last:?}"
    );
    assert_eq!(
        (last.width, last.height),
        (CANVAS, CANVAS),
        "told fullscreen without the output's size"
    );

    // The snapshot agents read carries it.
    let snapshots = fixture.state.window_snapshots();
    let snapshot = snapshots.iter().find(|w| w.id == fixture.id(0).0).unwrap();
    assert!(snapshot.fullscreen);
    assert!(
        !snapshots
            .iter()
            .find(|w| w.id == fixture.id(1).0)
            .unwrap()
            .fullscreen
    );

    // An unknown id is accepted and changes nothing.
    let before = fixture.state.world.arrange();
    let response =
        fixture
            .state
            .handle_request(Request::Action(scoot_ipc::Action::SetFullscreen {
                id: u64::MAX,
                fullscreen: true,
            }));
    assert!(matches!(response, Response::Ok { .. }), "{response:?}");
    assert_eq!(fixture.state.world.arrange(), before);
}

#[test]
fn a_locked_session_honours_the_window_s_own_request_and_refuses_everything_else() {
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.done(Step::LockSession);
    assert!(fixture.state.session_lock.is_locked());
    let id = fixture.id(0);

    // The bind and IPC are refused behind the lock...
    fixture.state.act(Action::ToggleFullscreen);
    assert!(!fixture.state.world.is_fullscreen(id));
    let response = fixture
        .state
        .handle_request(Request::Action(scoot_ipc::Action::ToggleFullscreen));
    assert!(matches!(response, Response::Error { .. }), "{response:?}");
    assert!(!fixture.state.world.is_fullscreen(id));

    // ...but the window's own request is its own state, and is answered.
    let entered = fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    assert!(entered.fullscreen, "{entered:?}");
    assert!(fixture.state.world.is_fullscreen(id));
}

#[test]
fn the_output_hint_carries_the_focused_window_to_that_output() {
    let mut fixture = Fixture::new();
    let second =
        crate::compositor::headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
            .expect("a second output");
    fixture.settle();
    fixture.map(WINDOW_BGRA);
    let first = fixture.state.outputs.primary_id().unwrap();
    assert_eq!(
        fixture
            .state
            .world
            .arrange()
            .get(fixture.id(0))
            .unwrap()
            .output,
        first
    );

    // The client's second `wl_output` is the second output (registry order).
    let entered = fixture.configured(Step::SetFullscreen {
        window: 0,
        output: Some(1),
    });
    assert!(entered.fullscreen, "{entered:?}");
    let placed = *fixture.state.world.arrange().get(fixture.id(0)).unwrap();
    assert_eq!(placed.output, second, "the hint was not honoured");
    assert_eq!(
        fixture.state.world.fullscreen_on(second),
        Some(fixture.id(0))
    );
    assert_eq!(fixture.state.world.fullscreen_on(first), None);
    // The second output sits right of the first.
    assert_eq!(placed.rect, Rect::new(CANVAS, 0, CANVAS, CANVAS));
}

#[test]
fn the_output_hint_is_ignored_for_a_window_that_is_not_focused() {
    let mut fixture = Fixture::new();
    crate::compositor::headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
        .expect("a second output");
    fixture.settle();
    fixture.map(WINDOW_BGRA);
    fixture.map(OTHER_BGRA);
    let first = fixture.state.outputs.primary_id().unwrap();

    let entered = fixture.configured(Step::SetFullscreen {
        window: 0,
        output: Some(1),
    });
    assert!(entered.fullscreen, "{entered:?}");
    assert_eq!(
        fixture
            .state
            .world
            .arrange()
            .get(fixture.id(0))
            .unwrap()
            .output,
        first,
        "an unfocused window was moved to another output on a client's say-so"
    );
    assert_eq!(fixture.state.focus, Some(fixture.id(1)));
}

#[test]
fn a_refused_request_is_answered_without_changing_the_window_s_size() {
    // A window stacked under a fullscreen sibling cannot enter (only a
    // column's focused window may be fullscreen). It is still answered,
    // with the bit clear -- and not resized to the sibling's frame, which
    // is what its own placement reads while it is hidden.
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.map(OTHER_BGRA);
    fixture
        .state
        .act(Action::ConsumeOrExpel(scoot_core::Horizontal::Left));
    fixture.state.act(Action::ToggleFullscreen);
    fixture.settle();
    assert!(fixture.state.world.is_fullscreen(fixture.id(1)));
    let before = *fixture.configures(0).last().unwrap();

    let answer = fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    assert!(!answer.fullscreen, "{answer:?}");
    assert_eq!((answer.width, answer.height), (before.width, before.height));
    assert!(!fixture.state.world.is_fullscreen(fixture.id(0)));
    assert!(fixture.state.world.is_fullscreen(fixture.id(1)));
}

#[test]
fn a_frame_the_window_refuses_to_shrink_below_is_still_learned() {
    // The positive half of the frame-race test above: pairing a commit with
    // the configure it acked must still *learn* from a real refusal. A
    // client that acks the tiled configure and then commits 150px wide has
    // a minimum the core did not know about, and its column widens to it.
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.map(OTHER_BGRA);
    let tiled = fixture.rect_of(1);
    assert_eq!(tiled.w, 82);

    fixture.done(Step::DrawSized {
        window: 1,
        width: 150,
        height: tiled.h,
    });
    let learned = fixture.rect_of(1);
    assert_eq!(learned.w, 150, "a real refusal to shrink was not learned");
    assert_eq!(learned.right(), tiled.right(), "{learned:?}");
}

#[test]
fn a_request_that_changes_nothing_is_answered_without_a_relayout() {
    // A client repeating `unset_fullscreen` on a tiled window (or
    // `set_fullscreen` on a fullscreen one) must still get its configure,
    // but not a full `apply()`. Observed by changing the core behind
    // `apply()`'s back: had the request run one, its answer would carry the
    // new size; the fast path answers with the size the window already has.
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    let before = *fixture.configures(0).last().unwrap();
    fixture.state.world.handle_action(Action::CycleColumnWidth);
    assert_ne!(fixture.rect_of(0).w, before.width);

    let answer = fixture.configured(Step::UnsetFullscreen { window: 0 });
    assert!(!answer.fullscreen);
    assert_eq!(
        (answer.width, answer.height),
        (before.width, before.height),
        "a no-op request ran a full relayout"
    );
}
