use super::*;
use crate::{Action, Vertical};

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
