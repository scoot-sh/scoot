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

#[test]
fn activating_the_focused_window_still_takes_the_keyboard_back() {
    // The half of that guard which must *not* be skipped. A taskbar is a layer
    // surface, and clicking one can leave it holding the keyboard (see
    // `layer_shell.rs`); `shell.rs`'s `set_focus` says its unconditional
    // `refresh_keyboard_focus` is the only thing that takes the keyboard back
    // when the window focus itself has not moved -- which is exactly the case
    // the guard above short-circuits.
    //
    // `keyboard_on_layer` is written by `refresh_keyboard_focus` and by nothing
    // else (its own doc says so), so pre-setting it and watching it be
    // re-derived is a direct observation that the refresh ran.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture.state.keyboard_on_layer = true;
    fixture.state.wlr_toplevel_activate(WindowId(1));
    assert!(
        !fixture.state.keyboard_on_layer,
        "activating the focused window left the keyboard on a layer surface"
    );
}

#[test]
fn activating_the_focused_window_while_locked_moves_no_keyboard_either() {
    // The lock gate covers both halves of `activate`, not just the one that
    // goes through `State::act`: while the session is locked the keyboard
    // belongs to the lock surface, and a client asking for a window must not
    // start a focus re-derivation at all.
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    fixture.run(Step::LockSession);
    fixture.take_log();
    assert!(fixture.state.session_lock.is_locked());

    fixture.state.keyboard_on_layer = true;
    fixture.state.wlr_toplevel_activate(WindowId(1));
    assert!(
        fixture.state.keyboard_on_layer,
        "a locked session re-derived keyboard focus for a foreign-toplevel activate"
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
    // the id it names must not resolve onto anything -- flexwm's window ids
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
    // halves have nothing in flexwm's core to attach to, and `set_rectangle`
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
