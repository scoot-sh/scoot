//! `output_enter`, and the bind-order problem it has.
//!
//! The event's argument is a `wl_output` *object*, which only exists for a
//! client that has bound one -- so what a handle can say about the screen
//! depends on what else that client has done, and on when. Registry order is
//! the server's choice, so every order below is something a real shell can
//! produce.
//!
//! The wrong-client case is the dangerous one: wayland-backend *panics* when
//! an event carries an object belonging to a different client than the one it
//! is sent to, and a panic in a compositor takes every client's session with
//! it.

use super::*;
use crate::compositor::test_support::Harness;

/// A live compositor with two side-by-side outputs and one connected client.
///
/// The extra output is added before the client connects, so the registry
/// announces both in creation order: `BindOutputAt(0)` is the primary, where
/// new windows open, and `BindOutputAt(1)` is the second.
fn two_output_fixture() -> Fixture {
    let mut fixture = Harness::headless(Appearance::default(), CANVAS);
    crate::compositor::headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
        .expect("a second headless output");
    fixture.spawn(run_client);
    fixture
}

/// The subset of a log that is about outputs, which is all these tests assert
/// on -- the rest of the announcement burst is `mod.rs`'s subject.
fn output_events(log: &[Seen]) -> Vec<Seen> {
    log.iter()
        .filter(|seen| matches!(seen, Seen::OutputEnter(_) | Seen::OutputLeave(_)))
        .cloned()
        .collect()
}

#[test]
fn a_window_is_announced_on_the_output_it_is_on() {
    let mut fixture = Fixture::bound();
    fixture.run(Step::MapWindow);
    assert_eq!(
        output_events(&fixture.take_log()),
        vec![Seen::OutputEnter(0)]
    );
}

#[test]
fn a_manager_bound_before_the_output_is_told_when_the_output_arrives() {
    // The order the `output_bound` hook exists for: a shell that binds this
    // manager before it binds the screen would otherwise show windows
    // belonging to no output, forever. The enter arrives in its own batch,
    // closed by `done`, once the client has an object to name.
    let mut fixture = Fixture::new();
    fixture.run(Step::BindManager);
    fixture.run(Step::MapWindow);
    let log = fixture.take_log();
    assert_eq!(
        output_events(&log),
        Vec::new(),
        "an output was named before the client had one: {log:?}"
    );

    fixture.run(Step::BindOutput);
    assert_eq!(
        fixture.take_log(),
        vec![Seen::OutputEnter(0), Seen::Done(0)],
    );
}

#[test]
fn every_handle_the_client_holds_is_told_when_it_binds_the_output() {
    // Two windows and two managers: the hook has to reach every handle that
    // client owns, not just the first one it finds.
    let mut fixture = Fixture::new();
    fixture.run(Step::BindManager);
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapWindow);
    fixture.run(Step::BindManager);
    fixture.take_log();

    fixture.run(Step::BindOutput);
    let log = fixture.take_log();
    assert_eq!(
        log.iter()
            .filter(|seen| matches!(seen, Seen::OutputEnter(_)))
            .count(),
        4,
        "not every handle was told about the output: {log:?}"
    );
}

#[test]
fn a_client_that_never_binds_the_output_is_served_anyway() {
    // Legal and unremarkable -- an alt-tab switcher has no reason to care
    // which screen a window is on. It must simply never be sent an
    // `output_enter`, and everything else must keep working.
    let mut fixture = Fixture::new();
    fixture.run(Step::BindManager);
    fixture.take_log();

    fixture.run(Step::MapWindow);
    fixture.run(Step::SetTitle(0, "no screen".to_string()));
    let log = fixture.take_log();
    assert_eq!(output_events(&log), Vec::new());
    assert!(
        log.contains(&Seen::Title(0, "no screen".to_string())),
        "a client with no wl_output stopped being served: {log:?}"
    );
    assert_eq!(fixture.tracked(), 1);
}

#[test]
fn a_client_that_bound_the_output_twice_is_told_on_each_object() {
    // The protocol's argument is an object, not an output, so a client holding
    // two `wl_output`s for the one screen is told on both -- which is what
    // wlroots does, and what a client demultiplexing by object expects.
    let mut fixture = Fixture::new();
    fixture.run(Step::BindOutput);
    fixture.run(Step::BindOutput);
    fixture.run(Step::BindManager);
    fixture.run(Step::MapWindow);
    let log = fixture.take_log();
    assert_eq!(
        output_events(&log),
        vec![Seen::OutputEnter(0), Seen::OutputEnter(0)],
        "a client with two wl_output objects was not told on both: {log:?}"
    );
}

#[test]
fn another_clients_output_bind_reaches_none_of_this_clients_handles() {
    // The compositor-killing case. `output_bound` fires for *every* client's
    // bind, and sending an event carrying another client's object panics
    // wayland-backend -- so this is both a correctness test (client 0 hears
    // nothing) and a liveness one (the compositor is still serving afterwards).
    let mut fixture = Fixture::new();
    fixture.spawn(run_client);
    fixture.run_on(0, Step::BindManager);
    fixture.run_on(0, Step::MapWindow);
    fixture.take_log();

    fixture.run_on(1, Step::BindOutput);
    let log = fixture.take_log();
    assert_eq!(
        output_events(&log),
        Vec::new(),
        "another client's wl_output was named on this client's handle: {log:?}"
    );

    // Still alive, still serving -- which is the other half of the assertion.
    fixture.run_on(0, Step::SetTitle(0, "survived".to_string()));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::Title(0, "survived".to_string()), Seen::Done(0)],
    );
}

#[test]
fn a_client_binding_the_output_with_no_handles_yet_is_harmless() {
    // The empty case, reached by every client that binds `wl_output` before it
    // has any windows to hear about -- i.e. nearly all of them.
    let mut fixture = Fixture::new();
    fixture.run(Step::BindOutput);
    fixture.run(Step::BindOutput);
    assert_eq!(fixture.take_log(), Vec::new());

    fixture.run(Step::BindManager);
    fixture.run(Step::MapWindow);
    assert_eq!(
        output_events(&fixture.take_log()),
        vec![Seen::OutputEnter(0), Seen::OutputEnter(0)],
    );
}

// ---------------------------------------------------------------------------
// More than one output: a window is announced on the output it is on
// ---------------------------------------------------------------------------

/// A window is announced with the output it is on, and a bind of any other
/// output stays silent. A window opened with the pointer on the first output
/// is announced there, so binding the first screen hears the enter and
/// binding the second hears nothing -- the same window, the same handle,
/// told per output rather than announced on the primary unconditionally.
#[test]
fn a_window_is_announced_on_its_own_output_and_no_other() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::BindOutputAt(0));
    fixture.run(Step::BindManager);
    fixture.take_log();

    fixture.run(Step::MapWindow);
    assert_eq!(
        output_events(&fixture.take_log()),
        vec![Seen::OutputEnter(0)],
        "a window on the first output was not announced on it"
    );

    // The second screen arrives late: no window is on it, so no enter.
    fixture.run(Step::BindOutputAt(1));
    assert_eq!(
        output_events(&fixture.take_log()),
        Vec::new(),
        "binding the second output announced a window that is not on it"
    );
}

/// A client holding only the second screen's `wl_output` is still served a
/// window on the first -- it just never gets an `output_enter` for an output
/// it has no object for.
#[test]
fn a_client_with_only_the_other_output_is_served_without_an_enter() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::BindOutputAt(1));
    fixture.run(Step::BindManager);
    fixture.take_log();

    fixture.run(Step::MapWindow);
    let log = fixture.take_log();
    assert_eq!(
        output_events(&log),
        Vec::new(),
        "a client with no first-output object was told an enter"
    );
    assert!(
        log.contains(&Seen::Toplevel(0)),
        "a client with no first-output object stopped being served: {log:?}"
    );
}

// ---------------------------------------------------------------------------
// Cross-output moves: `output_leave` pairing (milestone 19, phase F)
// ---------------------------------------------------------------------------

/// A window carried across outputs is told `output_leave` for the old screen
/// and `output_enter` for the new one, closed by `done` -- fail-first: with
/// no membership refresh the move sends nothing and the taskbar shows the
/// window on a screen it left.
///
/// The leave comes first: the protocol orders membership as "stops being
/// visible here, becomes visible there", and a client diffing the two needs
/// the pair in that order rather than a moment where the window is on both.
#[test]
fn a_window_moved_across_outputs_is_told_leave_then_enter() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::BindOutputAt(0));
    fixture.run(Step::BindOutputAt(1));
    fixture.run(Step::BindManager);
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture
        .state
        .act(Action::MoveFocusedWindowToOutput(OutputId(2)));
    fixture.settle();
    assert_eq!(
        fixture.take_log(),
        vec![Seen::OutputLeave(0), Seen::OutputEnter(0), Seen::Done(0)],
        "a cross-output move did not pair leave with enter"
    );
}

/// ...and back again: the stored membership follows the window rather than
/// sticking to the first move, so the second move pairs the other way round
/// instead of repeating (or dropping) the first pair.
#[test]
fn a_window_moved_back_is_told_leave_then_enter_again() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::BindOutputAt(0));
    fixture.run(Step::BindOutputAt(1));
    fixture.run(Step::BindManager);
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture
        .state
        .act(Action::MoveFocusedWindowToOutput(OutputId(2)));
    fixture.settle();
    fixture.take_log();

    fixture
        .state
        .act(Action::MoveFocusedWindowToOutput(OutputId(1)));
    fixture.settle();
    assert_eq!(
        fixture.take_log(),
        vec![Seen::OutputLeave(0), Seen::OutputEnter(0), Seen::Done(0)],
        "moving the window back did not pair leave with enter"
    );
}

/// Closing a window sends `closed` with no `leave` first: the handle's whole
/// death is the `closed` event, and nothing may be sent on it after -- a
/// leave-then-closed sequence would also break the protocol's guarantee that
/// a `leave` only ever follows an `enter` the client can still use.
#[test]
fn closing_a_window_sends_closed_without_a_leave() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::BindOutputAt(0));
    fixture.run(Step::BindOutputAt(1));
    fixture.run(Step::BindManager);
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture.run(Step::CloseWindow(0));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::Closed(0)],
        "closing a window paired its close with a leave"
    );
}

/// A client holding no `wl_output` for either screen hears nothing about the
/// move -- and in particular no bare `done`: with no object to name on either
/// side there is no batch to close, and a client that draws on `done` must
/// not redraw over a change it was never told.
#[test]
fn a_move_is_silent_to_a_client_with_no_output_objects() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::BindManager);
    fixture.run(Step::MapWindow);
    fixture.take_log();

    fixture
        .state
        .act(Action::MoveFocusedWindowToOutput(OutputId(2)));
    fixture.settle();
    assert_eq!(
        fixture.take_log(),
        Vec::new(),
        "a client with no wl_output was told about a move it cannot name"
    );
}

// ---------------------------------------------------------------------------
// New-window placement: the pointer's output (milestone 19, phase G)
// ---------------------------------------------------------------------------

/// A window opened with the pointer on the second output is announced on
/// that output -- fail-first: with `WindowOpened` filed on the first output
/// unconditionally, a client holding only the second screen's object hears
/// nothing, and the core places the window there.
#[test]
fn a_new_window_opens_on_the_pointers_output() {
    let mut fixture = two_output_fixture();
    // Only the second screen's object: an enter here names output 2, and
    // nothing else can produce one (`OutputEnter` is keyed by handle, so
    // the discrimination is which object hears it, not the event's value).
    fixture.run(Step::BindOutputAt(1));
    fixture.run(Step::BindManager);
    fixture.take_log();

    // Each output is CANVAS wide, so this point is on the second screen.
    fixture.state.pointer_move(f64::from(CANVAS) + 100.0, 100.0);
    fixture.run(Step::MapWindow);
    assert_eq!(
        output_events(&fixture.take_log()),
        vec![Seen::OutputEnter(0)],
        "a window opened with the pointer on output 2 was not announced on it"
    );
    let placements = fixture.state.world.arrange().placements;
    assert_eq!(
        placements.len(),
        1,
        "expected exactly the one mapped window: {placements:?}"
    );
    assert_eq!(
        placements[0].output,
        OutputId(2),
        "a window opened with the pointer on output 2 was not placed on it"
    );
    let snapshots = fixture.state.window_snapshots();
    assert_eq!(
        snapshots.len(),
        1,
        "expected exactly the one mapped window: {snapshots:?}"
    );
    assert_eq!(
        snapshots[0].output, 2,
        "the IPC `output` field does not name the output the window opened on"
    );
}

/// The mirror: a client holding only the first screen's object hears nothing
/// about that same window -- fail-first the other way round, since the old
/// code announced everything on the first output.
#[test]
fn a_new_window_on_the_second_output_is_silent_on_the_first() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::BindOutputAt(0));
    fixture.run(Step::BindManager);
    fixture.take_log();

    fixture.state.pointer_move(f64::from(CANVAS) + 100.0, 100.0);
    fixture.run(Step::MapWindow);
    let log = fixture.take_log();
    assert_eq!(
        output_events(&log),
        Vec::new(),
        "a window on output 2 was announced on the first output: {log:?}"
    );
    assert!(
        log.contains(&Seen::Toplevel(0)),
        "a client with no second-output object stopped being served: {log:?}"
    );
}

/// The same path with the pointer on the first output: the window opens
/// there -- the pre-G behavior, kept as a regression pin.
#[test]
fn a_new_window_opens_on_the_first_output_with_the_pointer_there() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::BindOutputAt(0));
    fixture.run(Step::BindOutputAt(1));
    fixture.run(Step::BindManager);
    fixture.take_log();

    fixture.state.pointer_move(100.0, 100.0);
    fixture.run(Step::MapWindow);
    assert_eq!(
        output_events(&fixture.take_log()),
        vec![Seen::OutputEnter(0)],
        "a window opened with the pointer on output 1 was not announced on it"
    );
    let placements = fixture.state.world.arrange().placements;
    assert_eq!(
        placements.len(),
        1,
        "expected exactly the one mapped window: {placements:?}"
    );
    assert_eq!(
        placements[0].output,
        OutputId(1),
        "a window opened with the pointer on output 1 was not placed on it"
    );
}

/// A window opened with the pointer over no output at all falls back to the
/// primary -- absolute motion is never clamped, so off-output (and, with
/// uneven outputs, dead-zone) pointer positions are reachable, and they must
/// still open somewhere rather than nowhere.
#[test]
fn a_new_window_with_the_pointer_over_no_output_falls_back_to_primary() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::BindOutputAt(0));
    fixture.run(Step::BindOutputAt(1));
    fixture.run(Step::BindManager);
    fixture.take_log();

    fixture.state.pointer_move(-50.0, -50.0);
    fixture.run(Step::MapWindow);
    assert_eq!(
        output_events(&fixture.take_log()),
        vec![Seen::OutputEnter(0)],
        "a window opened with the pointer over no output was not announced on the primary"
    );
    let placements = fixture.state.world.arrange().placements;
    assert_eq!(
        placements.len(),
        1,
        "expected exactly the one mapped window: {placements:?}"
    );
    assert_eq!(
        placements[0].output,
        OutputId(1),
        "a window opened with the pointer over no output did not fall back to the primary"
    );
}
