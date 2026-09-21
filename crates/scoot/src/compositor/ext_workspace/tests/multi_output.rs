//! Workspaces across more than one output (milestone 19, phase D).
//!
//! The per-output pins: binding announces one workspace group per output,
//! each carrying its own output and its own workspace list, and an `activate`
//! staged on one output's handle switches (or no-ops on) that output's
//! workspaces without disturbing any other's. The harm in scope -- a
//! workspace switch on one output moving another output's active workspace,
//! which would visibly jump a screen the user never asked about -- is pinned
//! fail-first below.
//!
//! Windows open on the first output throughout (nothing moves a window
//! across outputs yet -- that is a later phase), so the second output's list
//! is always the single empty workspace: its group is announced, stays
//! coherent, and every `activate` on it is a no-op that must not leak onto
//! the first output.
//!
//! Like the parent suite these drive a real `wayland-client` connection and
//! assert on the exact event sequence, in order. The canvas is square and
//! both outputs are the same size, placed side by side: output 1 covers
//! `0..CANVAS` on both axes, output 2 covers `CANVAS..2*CANVAS` on `x`.

use super::*;
use scoot_core::OutputId;

use crate::compositor::headless;

/// A live compositor with two side-by-side outputs and one connected client.
fn two_output_fixture() -> Fixture {
    let mut fixture = Harness::headless(Appearance::default(), CANVAS);
    headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
        .expect("a second output");
    fixture.spawn(run_client);
    fixture
}

/// What the core says one output's workspaces are right now, as
/// `(count, active)`.
fn workspaces_of(fixture: &Fixture, id: u64) -> (usize, usize) {
    let workspaces = fixture
        .state
        .world
        .workspaces(OutputId(id))
        .expect("the output exists");
    (workspaces.count, workspaces.active)
}

/// The first handle key the log entered into `group` -- what an `activate`
/// step addresses that group's workspace with.
fn handle_in_group(log: &[Seen], group: u32) -> usize {
    log.iter()
        .find_map(|seen| match seen {
            Seen::WorkspaceEnter(g, key) if *g == group => Some(*key as usize),
            _ => None,
        })
        .expect("the log should have entered a handle into that group")
}

/// A fixture bound to both outputs and the manager with the burst drained,
/// holding two windows on the first output's workspaces: window 1 on
/// workspace 1, window 2 on the active workspace 2. The second output is
/// untouched throughout.
fn two_windows_first_active() -> (Fixture, Vec<Seen>) {
    let mut fixture = two_output_fixture();
    fixture.run(Step::BindOutputAt(0));
    fixture.run(Step::BindOutputAt(1));
    fixture.run(Step::BindManager);
    let burst = fixture.take_log();
    fixture.run(Step::MapWindow); // window 1 on workspace 1
    fixture.take_log();
    // Onto the trailing empty workspace before mapping the second window,
    // so the two windows end up on different workspaces.
    fixture.act(Action::FocusWorkspace(Vertical::Down));
    fixture.run(Step::MapWindow); // window 2 on workspace 2, focused
    fixture.take_log();
    assert_eq!(
        workspaces_of(&fixture, 1),
        (3, 1),
        "the setup should leave three workspaces with the second active on output 1"
    );
    assert_eq!(
        workspaces_of(&fixture, 2),
        (1, 0),
        "the setup should leave output 2's single empty workspace alone"
    );
    (fixture, burst)
}

// -- what a client is told -------------------------------------------------

/// Binding with two outputs announces one group per output, each carrying
/// its own output and its own workspace list, closed by a single `done` --
/// fail-first: with the group pinned to the primary, the second output is
/// invisible to every workspace client.
#[test]
fn binding_announces_one_group_per_output() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::BindOutputAt(0));
    fixture.run(Step::BindOutputAt(1));
    fixture.run(Step::BindManager);

    let mut expected = vec![
        Seen::Group(0),
        Seen::GroupCapabilities(0, 0),
        Seen::OutputEnter(0),
    ];
    expected.extend(created(0, 0, 1, true));
    expected.extend(vec![
        Seen::Group(1),
        Seen::GroupCapabilities(1, 0),
        Seen::OutputEnter(1),
    ]);
    expected.extend(created(1, 1, 1, true));
    expected.push(Seen::Done(0));
    assert_eq!(fixture.take_log(), expected);
}

// -- switching -------------------------------------------------------------

/// The harm, fail-first: activating the second output's (already active)
/// workspace must not move the first output's. Before per-output groups the
/// only group is the primary's, so there is no second group to address --
/// and routing the request through the focused output's list would switch
/// output 1 from workspace 2 back to workspace 1 under the user's feet.
#[test]
fn activating_on_the_second_output_leaves_the_first_alone() {
    let (mut fixture, burst) = two_windows_first_active();
    let second = handle_in_group(&burst, 1);

    fixture.run(Step::Activate(second));
    fixture.run(Step::Commit(0));
    assert_eq!(
        fixture.take_log(),
        vec![],
        "an already-active activate stages nothing and commits nothing"
    );
    assert_eq!(
        workspaces_of(&fixture, 1),
        (3, 1),
        "output 1's active workspace must not move"
    );
    assert_eq!(
        workspaces_of(&fixture, 2),
        (1, 0),
        "output 2's workspace is unchanged"
    );
}

/// Switching the first output through its own group's handle still works --
/// the regression pin that proves the setup above is valid with or without
/// per-output groups.
#[test]
fn switching_the_first_output_through_its_own_group_still_works() {
    let (mut fixture, burst) = two_windows_first_active();
    let first = handle_in_group(&burst, 0);

    fixture.run(Step::Activate(first));
    fixture.run(Step::Commit(0));
    fixture.take_log();
    assert_eq!(
        workspaces_of(&fixture, 1),
        (3, 0),
        "output 1 switches to its first workspace"
    );
    assert_eq!(
        workspaces_of(&fixture, 2),
        (1, 0),
        "output 2's workspace is unchanged"
    );
}

// -- binding order and teardown --------------------------------------------

/// A `wl_output` bound after the manager enters only its own group: the
/// late bind of output 1 must not enter group 2, and vice versa.
#[test]
fn a_wl_output_bound_late_enters_only_its_own_group() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::BindManager);
    let burst = fixture.take_log();
    assert!(
        !burst
            .iter()
            .any(|seen| matches!(seen, Seen::OutputEnter(_))),
        "with no wl_output bound, no group has an output to enter: {burst:?}"
    );

    fixture.run(Step::BindOutputAt(0));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::OutputEnter(0), Seen::Done(0)],
        "output 1's bind enters only group 1"
    );
    fixture.run(Step::BindOutputAt(1));
    assert_eq!(
        fixture.take_log(),
        vec![Seen::OutputEnter(1), Seen::Done(0)],
        "output 2's bind enters only group 2"
    );
}

/// `stop` ends updates for both groups at once: nothing further is sent on
/// either group's handles, exactly as with one group.
#[test]
fn stop_ends_updates_for_both_groups() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::BindOutputAt(0));
    fixture.run(Step::BindOutputAt(1));
    fixture.run(Step::BindManager);
    fixture.take_log();

    fixture.run(Step::Stop(0));
    assert_eq!(fixture.take_log(), vec![Seen::Finished(0)]);
    fixture.run(Step::MapWindow);
    assert_eq!(
        fixture.take_log(),
        vec![],
        "a stopped manager hears about neither group's workspaces"
    );
    assert_eq!(
        fixture.registered_managers(),
        0,
        "the manager should be gone"
    );
}

// -- cross-output moves (milestone 19, phase F) ----------------------------

/// Moving a window across outputs reassigns no group to any output: each
/// group keeps the screen it was announced with, so no `output_leave` (and
/// no second `output_enter`) fires on any group -- only the workspace
/// membership events the move implies. The protocol's `output_leave` is for
/// an output *removed from a group*, which runtime output add/remove would
/// be, and that is out of scope: with fixed outputs it is unreachable.
#[test]
fn moving_a_window_across_outputs_reassigns_no_group() {
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
    let log = fixture.take_log();
    assert!(
        !log.iter()
            .any(|seen| matches!(seen, Seen::OutputLeave(_) | Seen::OutputEnter(_))),
        "a cross-output move reassigned a group's output: {log:?}"
    );
    // ...while the workspaces themselves did move: output 1 is back to its
    // single empty workspace, and output 2 holds the window on its active
    // workspace with the trailing empty one behind it.
    assert_eq!(workspaces_of(&fixture, 1), (1, 0));
    assert_eq!(workspaces_of(&fixture, 2), (2, 0));
}
