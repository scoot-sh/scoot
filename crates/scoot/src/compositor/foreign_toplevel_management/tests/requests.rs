//! The control half: `activate`, `close`, the requests that are accepted and
//! ignored, and what the session lock does to all of them.
//!
//! This is what makes the wlr protocol different from the `ext-` list next
//! door, and what makes a mistake here worse than a wrong row in a taskbar: an
//! `activate` resolved onto the wrong window focuses the wrong window, and a
//! `close` resolved onto the wrong one closes it.

use super::*;

#[test]
fn activate_focuses_the_named_window() {
    // What clicking a taskbar entry does. The second window has the focus
    // after being mapped; activating the first takes it back, and both are
    // told, each in its own batch.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow); // window 1 -> handle 0
    fixture.run(Step::MapWindow); // window 2 -> handle 1, focused
    fixture.take_log();
    assert_eq!(fixture.state.focus, Some(WindowId(2)));

    fixture.run(Step::Activate(0));
    assert_eq!(fixture.state.focus, Some(WindowId(1)));
    let log = fixture.take_log();
    assert!(
        log.contains(&Seen::State(0, activated())),
        "the activated window was not reported active: {log:?}"
    );
    assert!(
        log.contains(&Seen::State(1, Vec::new())),
        "the window that lost focus was not reported inactive: {log:?}"
    );
}

#[test]
fn a_taskbar_can_activate_a_window_it_does_not_own() {
    // The whole point of the protocol: the client sending `activate` is a
    // shell, and the window belongs to somebody else entirely.
    let mut fixture = Fixture::new();
    fixture.spawn(run_client);
    fixture.run_on(0, Step::BindOutput);
    fixture.run_on(0, Step::BindManager);
    fixture.run_on(1, Step::MapWindow); // window 1, client 1's
    fixture.run_on(1, Step::MapWindow); // window 2, client 1's, focused
    fixture.take_log();

    fixture.run_on(0, Step::Activate(0));
    assert_eq!(fixture.state.focus, Some(WindowId(1)));
}

#[test]
fn activating_the_window_that_is_already_focused_does_no_work_at_all() {
    // Not an optimisation: `State::act` runs a full `apply` -- an arrange, a
    // configure per window and a render request -- and a client may repeat
    // `activate` as fast as it can write to its socket. The guard is checked
    // directly rather than over the wire so the assertion is deterministic:
    // `needs_render` is set by `apply`'s `request_render` and by nothing else
    // on this path, and nothing dispatches in between.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow); // window 1
    fixture.run(Step::MapWindow); // window 2, focused
    fixture.take_log();

    fixture.state.needs_render = false;
    fixture.state.wlr_toplevel_activate(WindowId(2));
    assert!(
        !fixture.state.needs_render,
        "activating the already-focused window ran a full apply"
    );

    // ...and the contrast, so the assertion above is not passing for want of
    // anything happening at all.
    fixture.state.wlr_toplevel_activate(WindowId(1));
    assert!(
        fixture.state.needs_render,
        "activating a different window did not lay anything out"
    );
    assert_eq!(fixture.state.focus, Some(WindowId(1)));
}

/// Sets up the scenario every keyboard test below shares, and asserts it
/// really happened: one window with the keyboard, then a real `on_demand`
/// taskbar clicked so that it holds the keyboard instead.
///
/// The click has to be a real one through the pointer: `State::clicked_layer`
/// is written by `layer_shell.rs`'s `click_layer` and by nothing else, and it
/// is the field this whole group of tests is about.
fn taskbar_holding_the_keyboard() -> Fixture {
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapTaskbar);
    fixture.take_log();
    assert_eq!(
        fixture.keyboard_surface(),
        Some(fixture.window_surface(1)),
        "the window should start with the keyboard"
    );

    fixture.click(TASKBAR_POINT.0, TASKBAR_POINT.1);
    assert!(
        fixture.state.clicked_layer.is_some(),
        "the click never reached the taskbar -- check TASKBAR_POINT against the layout"
    );
    assert!(
        fixture.state.keyboard_on_layer,
        "an on_demand layer surface should hold the keyboard once clicked"
    );
    assert_ne!(
        fixture.keyboard_surface(),
        Some(fixture.window_surface(1)),
        "the window should have lost the keyboard to the taskbar"
    );
    assert_eq!(
        fixture.state.focus,
        Some(WindowId(1)),
        "clicking a bar must not move *window* focus"
    );
    fixture
}

#[test]
fn activating_the_focused_window_takes_the_keyboard_back_from_the_taskbar() {
    // The half of that guard which must *not* be skipped, and the exact
    // gesture: the user clicks a taskbar (which takes the keyboard, being
    // `on_demand`), then clicks the row of the window that already had focus.
    //
    // `refresh_keyboard_focus` alone is not enough and this test is what says
    // so: `layer_shell.rs`'s `layer_keyboard_focus` reads `clicked_layer` and
    // hands the keyboard straight back to a still-mapped `on_demand` surface,
    // so without clearing that field first the refresh re-derives the taskbar
    // and nothing moves. `input.rs`'s `focus_under_pointer` clears it on the
    // line before its own `act`, which is what makes clicking the window
    // itself work -- and what this path has to mirror.
    let mut fixture = taskbar_holding_the_keyboard();

    fixture.run(Step::Activate(0));

    assert_eq!(
        fixture.keyboard_surface(),
        Some(fixture.window_surface(1)),
        "activating the focused window left the keyboard on the taskbar"
    );
    assert!(!fixture.state.keyboard_on_layer);
    assert!(
        fixture.state.clicked_layer.is_none(),
        "the taskbar's click was not spent"
    );
    assert_eq!(fixture.state.focus, Some(WindowId(1)));
}

#[test]
fn activating_a_different_window_takes_the_keyboard_back_too() {
    // The other branch, which goes through `State::act` and so through
    // `set_focus`'s own `refresh_keyboard_focus` -- and which needs the same
    // `clicked_layer` clear for the same reason. Without it the *window* focus
    // would move while the keyboard stayed in the taskbar, which is a worse
    // state than either end of it.
    let mut fixture = taskbar_holding_the_keyboard();
    fixture.run(Step::MapWindow); // window 2, which takes focus
    fixture.click(TASKBAR_POINT.0, TASKBAR_POINT.1);
    assert!(
        fixture.state.keyboard_on_layer,
        "the taskbar has the keyboard"
    );
    fixture.take_log();

    // Handle 0 is window 1, which is *not* the focused one.
    fixture.run(Step::Activate(0));

    assert_eq!(fixture.state.focus, Some(WindowId(1)));
    assert_eq!(
        fixture.keyboard_surface(),
        Some(fixture.window_surface(1)),
        "activating another window left the keyboard on the taskbar"
    );
    assert!(fixture.state.clicked_layer.is_none());
}

#[test]
fn activating_the_focused_window_while_locked_moves_no_keyboard_either() {
    // The lock gate covers `activate` before either branch, not just the one
    // that goes through `State::act`: while the session is locked the keyboard
    // belongs to the lock surface, and a client asking for a window must not
    // start a focus re-derivation -- nor spend the taskbar's click, which has
    // to survive so the session comes back as the user left it.
    let mut fixture = taskbar_holding_the_keyboard();
    let taskbar = fixture
        .state
        .clicked_layer
        .clone()
        .expect("the taskbar holds the click");
    fixture.run(Step::LockSession);
    fixture.take_log();
    assert!(fixture.state.session_lock.is_locked());
    // The precondition this test is named for: without it a future change that
    // cleared window focus on lock would silently exercise the *other* branch.
    assert_eq!(fixture.state.focus, Some(WindowId(1)));

    fixture.run(Step::Activate(0));

    assert_ne!(
        fixture.keyboard_surface(),
        Some(fixture.window_surface(1)),
        "a locked session gave the keyboard to a window on a client's request"
    );
    assert_eq!(
        fixture.state.clicked_layer.as_ref(),
        Some(&taskbar),
        "a refused activate spent the taskbar's click anyway"
    );
}

#[test]
fn activating_the_focused_window_over_the_wire_tells_the_client_nothing() {
    // The same guard as seen by the client: no `state`, no `done`, no churn.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture.run(Step::Activate(0));
    fixture.run(Step::Activate(0));
    assert_eq!(fixture.take_log(), Vec::new());
    assert_eq!(fixture.state.focus, Some(WindowId(1)));
}

#[test]
fn activate_on_an_inert_handle_is_ignored() {
    // The race a real taskbar hits: the window closed while the click was in
    // flight. The protocol says an inert handle's requests are ignored, and
    // the id it names must not resolve onto anything -- scoot's window ids
    // never being reused is what guarantees it never resolves onto the *next*
    // window either.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow); // window 1 -> handle 0
    fixture.run(Step::MapWindow); // window 2 -> handle 1, focused
    fixture.run(Step::CloseWindow(0)); // window 1 is gone; handle 0 is inert
    fixture.take_log();

    fixture.run(Step::Activate(0));
    assert_eq!(fixture.take_log(), Vec::new());
    assert_eq!(fixture.state.focus, Some(WindowId(2)));
    assert_eq!(fixture.tracked(), 1);

    // And a window opened afterwards does not inherit the dead handle.
    fixture.run(Step::MapWindow); // window 3
    fixture.take_log();
    fixture.run(Step::Activate(0));
    assert_eq!(fixture.take_log(), Vec::new());
    assert_eq!(fixture.state.focus, Some(WindowId(3)));
}

#[test]
fn close_asks_the_window_to_close() {
    // The compositor does not destroy anything itself: it sends
    // `xdg_toplevel.close` and the window goes when its own client decides to.
    // Asserted from the window's client, which is the only place that event
    // can be seen.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture.run(Step::RequestClose(0));
    assert_eq!(fixture.take_log(), vec![Seen::AskedToClose(0)]);
    // Still there: nothing has closed it yet, and the protocol says so.
    assert_eq!(fixture.tracked(), 1);

    // ...and when the client does act on it, the handle is closed exactly once.
    fixture.run(Step::CloseWindow(0));
    assert_eq!(fixture.take_log(), vec![Seen::Closed(0)]);
    assert_eq!(fixture.tracked(), 0);
}

#[test]
fn close_names_the_right_window_among_several() {
    // The id lives in the handle's own user data, so this is the test that the
    // wrong window cannot be closed -- the worst thing this protocol could do.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow); // window 1 -> handle 0
    fixture.run(Step::MapWindow); // window 2 -> handle 1
    fixture.run(Step::MapWindow); // window 3 -> handle 2
    fixture.take_log();

    fixture.run(Step::RequestClose(1));
    assert_eq!(fixture.take_log(), vec![Seen::AskedToClose(1)]);
}

#[test]
fn close_on_an_inert_handle_is_ignored() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapWindow);
    fixture.run(Step::CloseWindow(0));
    fixture.take_log();

    fixture.run(Step::RequestClose(0));
    assert_eq!(
        fixture.take_log(),
        Vec::new(),
        "an inert handle's close reached a window"
    );
    assert_eq!(fixture.tracked(), 1);
}

#[test]
fn the_state_requests_are_accepted_and_do_nothing() {
    // `set_maximized`, `set_minimized`, `set_fullscreen` and their `unset_`
    // halves have nothing in scoot's core to attach to, and `set_rectangle`
    // is an animation hint this compositor reads nothing from -- including the
    // negative rectangle wlroots answers with an `invalid_rectangle` protocol
    // error. All of them must be survivable: killing a shell's connection over
    // a hint nothing looks at would be a worse answer than ignoring it, and a
    // taskbar's minimise button doing nothing is the documented behaviour.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture.run(Step::RequestIgnoredStates(0));
    assert_eq!(
        fixture.take_log(),
        Vec::new(),
        "an ignored request produced a state change"
    );

    // Still fully served afterwards -- the client was not disconnected and the
    // window is untouched.
    fixture.run(Step::SetTitle(0, "still here".to_string()));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::Title(0, "still here".to_string()), Seen::Done(0)],
    );
    assert_eq!(fixture.tracked(), 1);
}

#[test]
fn activate_and_close_are_refused_while_the_session_is_locked() {
    // The list itself stays live behind a lock screen (see `mod.rs`), but its
    // two requests are the same kind of thing every other requested action is,
    // and `shell.rs`'s gate exists because a window the user cannot see must
    // not be focused or closed from behind it.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow); // window 1 -> handle 0
    fixture.run(Step::MapWindow); // window 2 -> handle 1, focused
    fixture.run(Step::LockSession);
    fixture.take_log();
    assert!(fixture.state.session_lock.is_locked());

    fixture.run(Step::Activate(0));
    assert_eq!(
        fixture.state.focus,
        Some(WindowId(2)),
        "a locked session's focus was moved by a foreign-toplevel activate"
    );

    fixture.run(Step::RequestClose(0));
    assert_eq!(
        fixture.take_log(),
        Vec::new(),
        "a locked session's window was asked to close"
    );
    assert_eq!(fixture.tracked(), 2);
}
