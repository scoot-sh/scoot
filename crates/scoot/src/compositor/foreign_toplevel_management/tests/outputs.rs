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
/// output stays silent. New windows open on the first output, so binding the
/// first screen hears the enter and binding the second hears nothing -- the
/// same window, the same handle, told per output rather than announced on
/// the primary unconditionally.
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
