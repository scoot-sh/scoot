//! Restoring a reconnected output's workspaces (`World::evict_output` /
//! `World::restore_output`), and the positional output actions.

use super::*;
use crate::{Action, EvictedOutput, Vertical, Workspaces};

const SECOND: Rect = Rect::new(1000, 0, 800, 600);

fn add_output(world: &mut World, id: u64, area: Rect) {
    world.handle_event(Event::OutputAdded {
        id: OutputId(id),
        area,
    });
}

fn open_on(world: &mut World, id: u64, output: u64, focus: bool) {
    world.handle_event(Event::WindowOpened {
        id: WindowId(id),
        info: WindowInfo::default(),
        output: Some(OutputId(output)),
        focus,
    });
}

fn close(world: &mut World, id: u64) {
    world.handle_event(Event::WindowClosed { id: WindowId(id) });
}

/// Output 2 holding two workspaces -- ws0 `[10][13]`, ws1 `[11][12]` -- with
/// ws1 active and window 11 focused, output 1 holding window 1.
fn two_workspace_world() -> World {
    let mut world = world();
    add_output(&mut world, 2, SECOND);
    open(&mut world, 1);
    world.handle_action(Action::FocusOutput(OutputId(2)));
    open_on(&mut world, 10, 2, true);
    open_on(&mut world, 13, 2, true);
    open_on(&mut world, 11, 2, true);
    open_on(&mut world, 12, 2, true);
    // Carry 11 then 12 down into the trailing empty: ws1 `[11][12]`.
    world.handle_action(Action::FocusWindowId(WindowId(11)));
    world.handle_action(Action::MoveWindowToWorkspace(Vertical::Down));
    world.handle_action(Action::FocusWindowId(WindowId(12)));
    world.handle_action(Action::MoveWindowToWorkspace(Vertical::Down));
    world.handle_action(Action::FocusWindowId(WindowId(11)));
    assert_eq!(
        world.workspaces(OutputId(2)),
        Some(Workspaces {
            count: 3,
            active: 1
        }),
        "the setup holds two workspaces with the second active"
    );
    world
}

fn evict_second(world: &mut World) -> EvictedOutput {
    world.evict_output(OutputId(2)).expect("output 2 exists")
}

#[test]
fn evicting_reports_the_adopter_and_adopts_without_moving_focus() {
    let mut world = two_workspace_world();
    let evicted = evict_second(&mut world);
    assert_eq!(evicted.adopted_by, Some(OutputId(1)));
    assert_eq!(evicted.snapshot.workspaces.len(), 2);
    assert_eq!(evicted.snapshot.active, 1);
    // Everything moved to the adopter, and focus stayed on output 1's own
    // window -- removing output 2 must not steal it.
    for id in [10, 11, 12, 13] {
        assert_eq!(placement(&world, id).output, OutputId(1));
    }
}

#[test]
fn restoring_moves_the_still_open_windows_back_in_order() {
    let mut world = two_workspace_world();
    let evicted = evict_second(&mut world);
    add_output(&mut world, 3, SECOND);
    world.restore_output(OutputId(3), evicted);

    // The same workspaces, active index and column order.
    assert_eq!(
        world.workspaces(OutputId(3)),
        Some(Workspaces {
            count: 3,
            active: 1
        })
    );
    for id in [10, 11, 12, 13] {
        assert_eq!(placement(&world, id).output, OutputId(3), "window {id}");
    }
    // ws1 is active: its windows show, ws0's do not.
    assert!(placement(&world, 11).visible);
    assert!(placement(&world, 12).visible);
    assert!(!placement(&world, 10).visible);
    assert!(!placement(&world, 13).visible);
    // Column order within each workspace: left to right.
    assert!(placement(&world, 10).rect.x < placement(&world, 13).rect.x);
    assert!(placement(&world, 11).rect.x < placement(&world, 12).rect.x);
    // The adopter keeps only its own window, and focus never left it.
    assert_eq!(placement(&world, 1).output, OutputId(1));
    assert_eq!(world.focused_output(), Some(OutputId(1)));
}

#[test]
fn restoring_keeps_column_presets_and_focused_windows() {
    let mut world = two_workspace_world();
    world.handle_action(Action::FocusWindowId(WindowId(10)));
    world.handle_action(Action::CycleColumnWidth);
    let evicted = evict_second(&mut world);
    add_output(&mut world, 3, SECOND);
    world.restore_output(OutputId(3), evicted);

    let preset = world
        .outputs
        .iter()
        .flat_map(|output| &output.workspaces)
        .flat_map(|ws| &ws.columns)
        .find(|column| column.windows.contains(&WindowId(10)))
        .map(|column| column.preset)
        .expect("window 10 is in the tree");
    assert_eq!(preset, 1, "the column preset crossed the eviction");
    // Window 10 was focused on output 2; its restored workspace focuses it
    // again (without moving session focus off output 1).
    world.handle_action(Action::FocusOutput(OutputId(3)));
    assert_eq!(focused(&world), Some(10));
}

#[test]
fn a_window_moved_by_hand_to_another_workspace_stays() {
    let mut world = two_workspace_world();
    let evicted = evict_second(&mut world);
    // Carry 11 onto the adopter's own first workspace by hand.
    world.handle_action(Action::FocusWindowId(WindowId(11)));
    world.handle_action(Action::MoveWindowToWorkspaceIndex(0));
    assert_eq!(placement(&world, 11).output, OutputId(1));

    add_output(&mut world, 3, SECOND);
    world.restore_output(OutputId(3), evicted);

    assert_eq!(
        placement(&world, 11).output,
        OutputId(1),
        "the hand-moved window stays where it was put"
    );
    // Its old neighbours still go back, in order, around the gap.
    assert_eq!(placement(&world, 12).output, OutputId(3));
    assert_eq!(placement(&world, 10).output, OutputId(3));
    assert_eq!(placement(&world, 13).output, OutputId(3));
}

#[test]
fn a_window_moved_by_hand_to_another_output_stays() {
    let mut world = two_workspace_world();
    add_output(&mut world, 3, SECOND);
    // Adopt onto output 1 (not the just-added output 3): focus it first, so
    // the eviction's adopter is output 1.
    world.handle_action(Action::FocusOutput(OutputId(1)));
    let evicted = evict_second(&mut world);
    assert_eq!(evicted.adopted_by, Some(OutputId(1)));
    // Carry 11 onto the third output by hand.
    world.handle_action(Action::FocusWindowId(WindowId(11)));
    world.handle_action(Action::MoveFocusedWindowToOutput(OutputId(3)));
    assert_eq!(placement(&world, 11).output, OutputId(3));

    add_output(&mut world, 4, SECOND);
    world.restore_output(OutputId(4), evicted);

    assert_eq!(
        placement(&world, 11).output,
        OutputId(3),
        "the hand-moved window stays on the third output"
    );
    assert_eq!(placement(&world, 12).output, OutputId(4));
}

#[test]
fn a_closed_window_drops_out() {
    let mut world = two_workspace_world();
    let evicted = evict_second(&mut world);
    close(&mut world, 11);
    close(&mut world, 12);

    add_output(&mut world, 3, SECOND);
    world.restore_output(OutputId(3), evicted);

    // ws1 emptied out entirely, so it is gone; ws0 is back, and the active
    // index clamps onto the trailing empty -- the workspace the user was on
    // no longer exists, so they land on a fresh empty one, not on ws0's
    // windows unasked.
    assert_eq!(placement(&world, 10).output, OutputId(3));
    assert_eq!(placement(&world, 13).output, OutputId(3));
    assert_eq!(
        world.workspaces(OutputId(3)),
        Some(Workspaces {
            count: 2,
            active: 1
        })
    );
}

#[test]
fn restoring_an_empty_snapshot_or_onto_an_unknown_output_changes_nothing() {
    let mut world = two_workspace_world();
    // Output 1's snapshot is a single empty workspace... in fact output 1
    // holds window 1, so evict output 3 (added empty) for the empty case.
    add_output(&mut world, 3, SECOND);
    let empty = world.evict_output(OutputId(3)).expect("output 3 exists");
    assert!(empty.snapshot.workspaces.is_empty());
    let before = world.arrange();
    world.restore_output(OutputId(1), empty);
    assert_eq!(world.arrange(), before);

    // Unknown new output: the evicted record is dropped, nothing moves.
    let evicted = evict_second(&mut world);
    world.restore_output(OutputId(99), evicted);
    for id in [10, 11, 12, 13] {
        assert_eq!(placement(&world, id).output, OutputId(1));
    }
}

#[test]
fn restoring_onto_the_adopter_changes_nothing() {
    let mut world = two_workspace_world();
    let evicted = evict_second(&mut world);
    let before = world.arrange();
    world.restore_output(OutputId(1), evicted);
    assert_eq!(world.arrange(), before);
}

#[test]
fn a_floating_window_is_restored_floating() {
    let mut world = two_workspace_world();
    world.handle_action(Action::FocusWindowId(WindowId(10)));
    world.handle_action(Action::SetFloating {
        id: WindowId(10),
        floating: true,
    });
    assert!(placement(&world, 10).floating);
    let evicted = evict_second(&mut world);

    add_output(&mut world, 3, SECOND);
    world.restore_output(OutputId(3), evicted);

    let placed = placement(&world, 10);
    assert_eq!(placed.output, OutputId(3));
    assert!(placed.floating, "window 10 floats on the restored output");
}

#[test]
fn evicting_an_unknown_output_answers_none() {
    let mut world = world();
    assert_eq!(world.evict_output(OutputId(99)), None);
}

#[test]
fn evicting_the_last_output_parks_windows_for_the_next_one() {
    let mut world = world();
    open(&mut world, 1);
    let evicted = world.evict_output(OutputId(1)).expect("output 1 exists");
    assert_eq!(evicted.adopted_by, None);
    assert!(world.arrange().placements.is_empty());

    add_output(&mut world, 2, SECOND);
    world.restore_output(OutputId(2), evicted);
    assert_eq!(placement(&world, 1).output, OutputId(2));
}

// -- positional output actions --------------------------------------------

#[test]
fn focus_output_index_reaches_outputs_by_position() {
    let mut world = two_workspace_world();
    world.handle_action(Action::FocusOutputIndex(1));
    assert_eq!(world.focused_output(), Some(OutputId(2)));
    assert_eq!(focused(&world), Some(11));
    world.handle_action(Action::FocusOutputIndex(0));
    assert_eq!(world.focused_output(), Some(OutputId(1)));
    assert_eq!(focused(&world), Some(1));
    // Out of range does nothing, like an unknown id.
    world.handle_action(Action::FocusOutputIndex(7));
    assert_eq!(world.focused_output(), Some(OutputId(1)));
    assert_eq!(focused(&world), Some(1));
}

#[test]
fn move_focused_window_to_output_index_carries_and_follows() {
    let mut world = two_workspace_world();
    world.handle_action(Action::FocusWindowId(WindowId(1)));
    world.handle_action(Action::MoveFocusedWindowToOutputIndex(1));
    assert_eq!(placement(&world, 1).output, OutputId(2));
    assert_eq!(focused(&world), Some(1));
    assert_eq!(world.focused_output(), Some(OutputId(2)));
    // Out of range leaves the window where it is.
    world.handle_action(Action::MoveFocusedWindowToOutputIndex(7));
    assert_eq!(placement(&world, 1).output, OutputId(2));
}

#[test]
fn output_at_reports_creation_order() {
    let world = two_workspace_world();
    assert_eq!(world.output_at(0), Some(OutputId(1)));
    assert_eq!(world.output_at(1), Some(OutputId(2)));
    assert_eq!(world.output_at(2), None);
}
