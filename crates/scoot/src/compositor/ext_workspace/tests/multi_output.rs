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
use scoot_core::WindowId;

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

/// The handle key entered into `group` for its `position`-th workspace
/// (0-based, in announcement order) -- what an `activate` step addresses
/// that workspace with.
fn handle_at(log: &[Seen], group: u32, position: usize) -> usize {
    log.iter()
        .filter_map(|seen| match seen {
            Seen::WorkspaceEnter(g, key) if *g == group => Some(*key as usize),
            _ => None,
        })
        .nth(position)
        .expect("the log should have entered that workspace into that group")
}

/// What IPC `windows` reports right now. A `windows` request arranges
/// read-only, so it adds nothing to the protocol log above.
fn windows(fixture: &mut Fixture) -> Vec<scoot_ipc::WindowSnapshot> {
    match fixture.state.handle_request(scoot_ipc::Request::Windows) {
        scoot_ipc::Response::Windows { windows } => windows,
        other => panic!("expected windows, got {other:?}"),
    }
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
    let second = handle_at(&burst, 1, 0);

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
    // ...but focus follows the click across outputs anyway: a bar click is
    // an interaction with that monitor, the way a click on a window focuses
    // its output -- which is why the already-active case still goes through
    // the targeted action instead of the no-op fast path.
    assert_eq!(
        fixture.state.world.focused_output(),
        Some(OutputId(2)),
        "activating another output's workspace moves focus there even when nothing switches"
    );
    assert_eq!(
        fixture.state.focus, None,
        "output 2's active workspace holds no window"
    );
}

/// Switching the first output through its own group's handle still works --
/// the regression pin that proves the setup above is valid with or without
/// per-output groups.
#[test]
fn switching_the_first_output_through_its_own_group_still_works() {
    let (mut fixture, burst) = two_windows_first_active();
    let first = handle_at(&burst, 0, 0);

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

/// A fixture with both windows on the second output -- window 1 on its
/// first workspace, window 2 on its active second -- and focus back on the
/// first output, which holds no windows: an `activate` on the second
/// output's group is then a genuine cross-output switch, never the
/// focused-output path.
fn two_windows_second_active() -> (Fixture, Vec<Seen>) {
    let mut fixture = two_output_fixture();
    fixture.run(Step::BindOutputAt(0));
    fixture.run(Step::BindOutputAt(1));
    fixture.run(Step::BindManager);
    let mut log = fixture.take_log();
    // Windows open on the pointer's output, so the pointer moves across
    // first (a focus change alone would not move it).
    fixture.state.pointer_move(CANVAS as f64 + 10.0, 10.0);
    fixture.settle();
    log.extend(fixture.take_log());
    fixture.run(Step::MapWindow); // window 1 on output 2's workspace 1
    log.extend(fixture.take_log());
    // Onto the trailing empty workspace before mapping the second window,
    // so the two windows end up on different workspaces.
    fixture.act(Action::FocusWorkspace(Vertical::Down));
    log.extend(fixture.take_log());
    fixture.run(Step::MapWindow); // window 2 on output 2's workspace 2, focused
    log.extend(fixture.take_log());
    assert_eq!(
        workspaces_of(&fixture, 2),
        (3, 1),
        "the setup should leave three workspaces with the second active on output 2"
    );
    // Focus back on the first output: the switch below moves focus across
    // outputs rather than along the focused one.
    fixture.act(Action::FocusOutput(OutputId(1)));
    log.extend(fixture.take_log());
    assert_eq!(
        fixture.state.world.focused_output(),
        Some(OutputId(1)),
        "focus should be back on the first output"
    );
    (fixture, log)
}

/// The route this ticket builds: an `activate` on a non-focused output's
/// group switches that output's workspaces through the output-targeted
/// action -- never the focused output's list -- and focus follows it
/// there, the way a click on a window focuses its output.
#[test]
fn activating_a_workspace_on_the_second_output_switches_it_and_moves_focus() {
    let (mut fixture, log) = two_windows_second_active();
    let first = handle_at(&log, 1, 0);
    let second = handle_at(&log, 1, 1);

    fixture.run(Step::Activate(first));
    fixture.run(Step::Commit(0));
    assert_eq!(
        workspaces_of(&fixture, 2),
        (3, 0),
        "output 2 switches to its first workspace"
    );
    assert_eq!(
        workspaces_of(&fixture, 1),
        (1, 0),
        "output 1's workspace is unchanged"
    );
    assert_eq!(
        fixture.state.world.focused_output(),
        Some(OutputId(2)),
        "the switch moves focus to the output that was clicked"
    );
    assert_eq!(
        fixture.state.focus,
        Some(WindowId(1)),
        "focus lands on the activated workspace's window"
    );
    // The client is told in one batch: the old workspace off, the new one
    // on.
    let (first, second) = (first as u32, second as u32);
    assert_eq!(
        fixture.take_log(),
        &[
            Seen::State(second, INACTIVE),
            Seen::State(first, ACTIVE),
            Seen::Done(0),
        ]
    );
}

/// The lock gate first: a cross-output switch staged and committed behind
/// the lock screen must disturb nothing -- neither output's list, nor
/// focus -- and the client is told nothing.
#[test]
fn an_activate_on_the_second_output_while_locked_is_refused() {
    let (mut fixture, log) = two_windows_second_active();
    let target = handle_at(&log, 1, 0);

    let Ack::Locked = fixture.run(Step::LockSession) else {
        panic!("the client never took the session lock");
    };
    fixture.take_log();
    assert!(fixture.state.session_lock.is_locked());

    fixture.run(Step::Activate(target));
    fixture.run(Step::Commit(0));
    assert_eq!(
        workspaces_of(&fixture, 2),
        (3, 1),
        "a locked session's workspaces must not move"
    );
    assert_eq!(
        workspaces_of(&fixture, 1),
        (1, 0),
        "the other output's workspaces must not move either"
    );
    assert_eq!(
        fixture.state.world.focused_output(),
        Some(OutputId(1)),
        "a locked session's focus must not move"
    );
    assert_eq!(
        fixture.take_log(),
        vec![],
        "a refused activate announces nothing"
    );
}

/// The race the protocol's own batching creates, on the targeted path: the
/// client acts on the list it last saw, and the compositor decides against
/// the list it has. Output 2 holds one window plus its trailing empty
/// workspace; the client stages the trailing one, then the window goes
/// away and takes the list's second workspace with it.
#[test]
fn an_activate_that_goes_stale_before_commit_is_ignored_on_the_targeted_path() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::BindOutputAt(0));
    fixture.run(Step::BindOutputAt(1));
    fixture.run(Step::BindManager);
    let mut log = fixture.take_log();
    fixture.state.pointer_move(CANVAS as f64 + 10.0, 10.0);
    fixture.settle();
    log.extend(fixture.take_log());
    fixture.run(Step::MapWindow);
    log.extend(fixture.take_log());
    assert_eq!(workspaces_of(&fixture, 2), (2, 0));
    let trailing = handle_at(&log, 1, 1);

    fixture.run(Step::Activate(trailing));
    // The window goes away, so the staged workspace does too -- while an
    // `activate` for it is still staged.
    fixture.run(Step::CloseWindow(0));
    fixture.run(Step::Commit(0));
    assert_eq!(workspaces_of(&fixture, 2), (1, 0));
    assert_eq!(workspaces_of(&fixture, 1), (1, 0));
    assert_eq!(
        fixture.state.world.focused_output(),
        Some(OutputId(2)),
        "the close keeps focus where it was; the refused activate moves nothing"
    );
    // Only the removal, and nothing from the stale activate.
    let gone = trailing as u32;
    assert_eq!(
        fixture.take_log(),
        &[
            Seen::WorkspaceLeave(1, gone),
            Seen::Removed(gone),
            Seen::Done(0),
        ]
    );
}

/// The output goes away between `activate` and `commit` (a `--tty` monitor
/// unplugged mid-gesture): the staged request names a group that no longer
/// exists, so the commit finds nothing to apply -- and the adoption the
/// removal filed stays exactly as it was.
#[test]
fn an_activate_on_a_removed_output_dies_with_its_group() {
    let (mut fixture, log) = two_windows_second_active();
    let target = handle_at(&log, 1, 0);

    fixture.run(Step::Activate(target));
    assert!(fixture.state.remove_output(OutputId(2)));
    fixture.settle();
    // The adoption the removal filed: both windows on output 1 now.
    for window in windows(&mut fixture) {
        assert_eq!(window.output, 1, "the removal's adoption is disturbed");
    }
    let adopted = workspaces_of(&fixture, 1);
    fixture.take_log();

    fixture.run(Step::Commit(0));
    assert_eq!(
        workspaces_of(&fixture, 1),
        adopted,
        "the dead request switched nothing"
    );
    assert_eq!(
        fixture.take_log(),
        vec![],
        "the dead request announced nothing"
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

// -- adopted workspaces carry their origin name -----------------------------

// A fixture with window A on output 1 and window B focused on output 2, and
// a bound manager that has seen everything so far: the shape an unplug
// starts from.
fn adopted_fixture() -> Fixture {
    let mut fixture = two_output_fixture();
    fixture.run(Step::BindOutputAt(0));
    fixture.run(Step::BindOutputAt(1));
    fixture.run(Step::BindManager);
    fixture.take_log();
    // Window A on output 1's first workspace...
    fixture.run(Step::MapWindow);
    fixture.take_log();
    // ...window B on output 2's, focused: the unplug below switches to it.
    // Windows open on the pointer's output, so the pointer moves across
    // first (a focus change alone would not move it).
    fixture.state.pointer_move(CANVAS as f64 + 10.0, 10.0);
    fixture.settle();
    fixture.take_log();
    fixture.run(Step::MapWindow);
    fixture.take_log();
    assert_eq!(workspaces_of(&fixture, 1), (2, 0));
    assert_eq!(workspaces_of(&fixture, 2), (2, 0));
    assert_eq!(fixture.state.world.focused_output(), Some(OutputId(2)));
    fixture
}

/// An unplug with focus on the removed output shows the adopted work under
/// its origin name: output 1's list grows "1 2" -> "1 2 headless-2 3" with
/// the active marker following the adopted workspace.
#[test]
fn unplugging_with_focus_there_names_the_adopted_workspace_for_its_monitor() {
    let mut fixture = adopted_fixture();

    assert!(fixture.state.remove_output(OutputId(2)));
    fixture.settle();
    assert_eq!(workspaces_of(&fixture, 1), (3, 1));
    // Handles so far: 0 and 1 are the two groups' first workspaces, 2 is
    // output 1's trailing empty, 3 is output 2's. The adopt adds output 1's
    // new trailing empty as handle 4.
    assert_eq!(
        fixture.take_log(),
        vec![
            Seen::WorkspaceLeave(1, 1),
            Seen::Removed(1),
            Seen::WorkspaceLeave(1, 3),
            Seen::Removed(3),
            Seen::GroupRemoved(1),
            Seen::Done(0),
            Seen::State(0, INACTIVE),
            Seen::State(2, ACTIVE),
            Seen::Workspace(4),
            Seen::Name(4, "3".into()),
            Seen::Coordinates(4, vec![3]),
            Seen::Capabilities(4, 1),
            Seen::State(4, INACTIVE),
            Seen::WorkspaceEnter(0, 4),
            Seen::Name(2, "2 headless-2".into()),
            Seen::Done(0),
        ]
    );
    // IPC agrees, in 0-based workspaces: window B sits on workspace 1,
    // adopted from headless-2; window A on workspace 0, never adopted.
    let listed = windows(&mut fixture);
    assert_eq!(listed.len(), 2);
    let adopted = listed
        .iter()
        .find(|w| w.workspace == 1)
        .expect("one window on the adopted workspace");
    assert_eq!(adopted.output, 1);
    assert!(adopted.adopted);
    assert_eq!(adopted.origin.as_deref(), Some("headless-2"));
    let own = listed
        .iter()
        .find(|w| w.workspace == 0)
        .expect("one window on the adopter's own workspace");
    assert_eq!(own.output, 1);
    assert!(!own.adopted);
    assert_eq!(own.origin, None);
}

/// Closing the adopter's own workspace renumbers the adopted one past it:
/// the handle keeps its position, so its name is re-sent following the
/// content ("2 headless-2" -> "1 headless-2", and the trailing "3" -> "2").
#[test]
fn closing_in_front_of_an_adopted_workspace_renames_it() {
    let mut fixture = adopted_fixture();
    assert!(fixture.state.remove_output(OutputId(2)));
    fixture.settle();
    fixture.take_log();

    // Window A is the client's first window: closing it empties output 1's
    // first workspace, which drops away under the adopted one.
    fixture.run(Step::CloseWindow(0));
    fixture.settle();
    assert_eq!(workspaces_of(&fixture, 1), (2, 0));
    assert_eq!(
        fixture.take_log(),
        vec![
            Seen::State(2, INACTIVE),
            Seen::State(0, ACTIVE),
            Seen::WorkspaceLeave(0, 4),
            Seen::Removed(4),
            Seen::Name(0, "1 headless-2".into()),
            Seen::Name(2, "2".into()),
            Seen::Done(0),
        ]
    );
}

/// A replug moves the adopted workspace home and takes the origin names back
/// off: the returned monitor's group is announced with bare positions, and
/// the adopter's trailing handle loses its tag.
#[test]
fn replugging_restores_the_workspace_and_clears_its_origin_name() {
    let mut fixture = adopted_fixture();
    assert!(fixture.state.remove_output(OutputId(2)));
    fixture.settle();
    fixture.take_log();

    headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
        .expect("the monitor comes back");
    fixture.settle();
    // Window B is home on the new output's first workspace; the adopter is
    // back on its own window, and focus followed B home.
    assert_eq!(workspaces_of(&fixture, 3), (2, 0));
    assert_eq!(workspaces_of(&fixture, 1), (2, 0));
    assert_eq!(fixture.state.world.focused_output(), Some(OutputId(3)));
    // Handles 5 and 6 are the returned output's group: announced with bare
    // positions, never tagged -- they are home, not adopted. The add
    // announces the empty group first; the restore then grows it and takes
    // the adopter's tag back off, each in its own batch.
    assert_eq!(
        fixture.take_log(),
        vec![
            Seen::Group(2),
            Seen::GroupCapabilities(2, 0),
            Seen::Workspace(5),
            Seen::Name(5, "1".into()),
            Seen::Coordinates(5, vec![1]),
            Seen::Capabilities(5, 1),
            Seen::State(5, ACTIVE),
            Seen::WorkspaceEnter(2, 5),
            Seen::Done(0),
            Seen::State(2, INACTIVE),
            Seen::State(0, ACTIVE),
            Seen::WorkspaceLeave(0, 4),
            Seen::Removed(4),
            Seen::Name(2, "2".into()),
            Seen::Workspace(6),
            Seen::Name(6, "2".into()),
            Seen::Coordinates(6, vec![2]),
            Seen::Capabilities(6, 1),
            Seen::State(6, INACTIVE),
            Seen::WorkspaceEnter(2, 6),
            Seen::Done(0),
        ]
    );
    // IPC agrees: window B is home on output 3's workspace 0, no longer
    // adopted; window A never left output 1's workspace 0.
    let listed = windows(&mut fixture);
    assert_eq!(listed.len(), 2);
    let home = listed
        .iter()
        .find(|w| w.output == 3)
        .expect("one window back on the returned monitor");
    assert_eq!(home.workspace, 0);
    assert!(!home.adopted);
    assert_eq!(home.origin, None);
    let stayed = listed
        .iter()
        .find(|w| w.output == 1)
        .expect("one window still on the first output");
    assert_eq!(stayed.workspace, 0);
    assert!(!stayed.adopted);
    assert_eq!(stayed.origin, None);
}

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

/// An output taken away (a `--tty` monitor unplugged) removes its group the
/// way `ext-workspace-v1` requires: each of its workspaces leaves the group
/// and is removed, then the group is removed -- and the other output's group
/// is left alone.
#[test]
fn removing_an_output_removes_its_group_in_protocol_order() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::BindOutputAt(0));
    fixture.run(Step::BindOutputAt(1));
    fixture.run(Step::BindManager);
    let burst = fixture.take_log();
    let handle = handle_at(&burst, 1, 0) as u32;

    assert!(fixture.state.remove_output(OutputId(2)));
    fixture.settle();
    let log = fixture.take_log();
    let leave = log
        .iter()
        .position(|seen| *seen == Seen::WorkspaceLeave(1, handle))
        .expect("the workspace leaves the removed group");
    let removed = log
        .iter()
        .position(|seen| *seen == Seen::Removed(handle))
        .expect("the workspace is removed");
    let group_removed = log
        .iter()
        .position(|seen| *seen == Seen::GroupRemoved(1))
        .expect("the group is removed");
    assert!(leave < removed && removed < group_removed, "{log:?}");
    assert!(
        !log.contains(&Seen::GroupRemoved(0)),
        "the remaining output's group stays: {log:?}"
    );
    assert!(
        matches!(log.get(group_removed + 1), Some(Seen::Done(_))),
        "a done closes the removal: {log:?}"
    );
}
