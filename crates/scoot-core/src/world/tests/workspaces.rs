use super::*;
use crate::{Action, Vertical, Workspaces};

#[test]
fn workspaces_always_end_in_one_empty_workspace() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    assert_eq!(workspace_count(&world), 2);

    world.handle_action(Action::MoveWindowToWorkspace(Vertical::Down));
    assert_eq!(workspace_count(&world), 3);
    assert_eq!(focused(&world), Some(2));
    assert!(placement(&world, 2).visible);
    assert!(!placement(&world, 1).visible);

    world.handle_action(Action::FocusWorkspace(Vertical::Up));
    assert_eq!(focused(&world), Some(1));
    assert!(!placement(&world, 2).visible);

    world.handle_action(Action::FocusWorkspace(Vertical::Down));
    world.handle_action(Action::FocusWorkspace(Vertical::Down));
    assert_eq!(focused(&world), None);
    assert_eq!(workspace_count(&world), 3);

    world.handle_action(Action::FocusWorkspace(Vertical::Up));
    assert_eq!(focused(&world), Some(2));
    assert_eq!(workspace_count(&world), 3);
}

// -- `World::workspaces`, the read side ----------------------------------

#[test]
fn an_empty_output_reports_one_empty_active_workspace() {
    let world = world();
    assert_eq!(
        world.workspaces(OutputId(1)),
        Some(Workspaces {
            count: 1,
            active: 0
        })
    );
}

#[test]
fn an_unknown_output_has_no_workspaces() {
    let world = world();
    assert_eq!(world.workspaces(OutputId(2)), None);
}

#[test]
fn the_reported_count_and_active_index_follow_the_tree() {
    let mut world = world();
    open(&mut world, 1);
    // One window, so: the workspace holding it plus the trailing empty one.
    assert_eq!(
        world.workspaces(OutputId(1)),
        Some(Workspaces {
            count: 2,
            active: 0
        })
    );

    world.handle_action(Action::FocusWorkspace(Vertical::Down));
    assert_eq!(
        world.workspaces(OutputId(1)),
        Some(Workspaces {
            count: 2,
            active: 1
        })
    );

    // Carrying the window down empties the first workspace, which is then
    // dropped -- so the count stays at two and the active index comes back to
    // zero even though the user moved "down". This renumbering is exactly why
    // an index is a position and not an identity.
    world.handle_action(Action::FocusWorkspace(Vertical::Up));
    world.handle_action(Action::MoveWindowToWorkspace(Vertical::Down));
    assert_eq!(
        world.workspaces(OutputId(1)),
        Some(Workspaces {
            count: 2,
            active: 0
        })
    );
    assert_eq!(focused(&world), Some(1));
}

// -- `Action::FocusWorkspaceIndex`, the write side -----------------------

#[test]
fn focusing_a_workspace_by_index_activates_exactly_it() {
    let mut world = world();
    open(&mut world, 1);
    world.handle_action(Action::FocusWorkspace(Vertical::Down));
    open(&mut world, 2);
    // Two occupied workspaces plus the trailing empty one.
    assert_eq!(workspace_count(&world), 3);

    world.handle_action(Action::FocusWorkspaceIndex(0));
    assert_eq!(focused(&world), Some(1));
    world.handle_action(Action::FocusWorkspaceIndex(1));
    assert_eq!(focused(&world), Some(2));
    // The trailing empty one is a real workspace a user can switch to.
    world.handle_action(Action::FocusWorkspaceIndex(2));
    assert_eq!(focused(&world), None);
    assert_eq!(
        world.workspaces(OutputId(1)),
        Some(Workspaces {
            count: 3,
            active: 2
        })
    );
}

#[test]
fn focusing_the_already_active_workspace_changes_nothing() {
    let mut world = world();
    open(&mut world, 1);
    let before = world.workspaces(OutputId(1));
    world.handle_action(Action::FocusWorkspaceIndex(0));
    assert_eq!(world.workspaces(OutputId(1)), before);
    assert_eq!(focused(&world), Some(1));
}

#[test]
fn an_out_of_range_workspace_index_is_ignored() {
    let mut world = world();
    open(&mut world, 1);
    let before = world.workspaces(OutputId(1));
    // One past the end, and the two extremes a client could send over a
    // protocol that takes an unbounded number.
    for index in [2, usize::MAX / 2, usize::MAX] {
        world.handle_action(Action::FocusWorkspaceIndex(index));
        assert_eq!(world.workspaces(OutputId(1)), before, "index {index}");
        assert_eq!(focused(&world), Some(1), "index {index}");
    }
}

#[test]
fn focusing_a_workspace_by_index_with_no_outputs_does_nothing() {
    // Nothing to index into: the action must not reach a tree that isn't
    // there. `reshape` already guards this, but it is the shape a protocol
    // client can provoke (bind, then the output goes away).
    let mut world = World::new(config());
    world.handle_action(Action::FocusWorkspaceIndex(0));
    assert_eq!(world.workspaces(OutputId(1)), None);
}

#[test]
fn leaving_an_emptied_workspace_by_index_renumbers_the_rest() {
    let mut world = world();
    open(&mut world, 1);
    world.handle_action(Action::FocusWorkspace(Vertical::Down));
    open(&mut world, 2);
    world.handle_action(Action::FocusWorkspace(Vertical::Down));
    open(&mut world, 3);
    assert_eq!(workspace_count(&world), 4);
    world.handle_action(Action::FocusWorkspaceIndex(1));

    // Closing the middle workspace's only window leaves it empty but active,
    // so it survives...
    world.handle_event(Event::WindowClosed { id: WindowId(2) });
    assert_eq!(
        world.workspaces(OutputId(1)),
        Some(Workspaces {
            count: 4,
            active: 1
        })
    );
    // ...until something else is activated, at which point it is dropped and
    // the workspace that was at index 2 is now at index 1.
    world.handle_action(Action::FocusWorkspaceIndex(2));
    assert_eq!(
        world.workspaces(OutputId(1)),
        Some(Workspaces {
            count: 3,
            active: 1
        })
    );
    assert_eq!(focused(&world), Some(3));
}

#[test]
fn observed_focus_switches_to_that_windows_workspace() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::MoveWindowToWorkspace(Vertical::Down));
    world.handle_event(Event::FocusObserved { id: WindowId(1) });
    assert_eq!(focused(&world), Some(1));
    assert!(placement(&world, 1).visible);
}
