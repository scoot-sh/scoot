//! Restoring a reconnected output's workspaces (`World::evict_output` /
//! `World::restore_output`), and the positional output actions.

use super::*;
use crate::{Action, AdopterView, EvictedOutput, Vertical, Workspaces};

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
fn evicting_with_focus_on_the_removed_output_shows_the_focused_work() {
    let mut world = two_workspace_world();
    assert_eq!(focused(&world), Some(11));
    let evicted = evict_second(&mut world);
    assert_eq!(evicted.adopted_by, Some(OutputId(1)));
    assert_eq!(evicted.snapshot.workspaces.len(), 2);
    assert_eq!(evicted.snapshot.active, 1);
    // The adopted block starts after the adopter's own workspace, tagged
    // with a fresh origin, and the adopter's view is one workspace before
    // the block.
    assert_eq!(evicted.adopted_at, 1);
    let origin = evicted.origin.expect("a non-empty adoption is tagged");
    assert_eq!(evicted.adopter_active, Some(AdopterView::BeforeBlock(1)));
    for id in [10, 11, 12, 13] {
        assert_eq!(placement(&world, id).output, OutputId(1));
    }
    // The switch: the adopter shows the adopted workspace that held window
    // 11, and focus stays on window 11.
    assert_eq!(
        world.workspaces(OutputId(1)),
        Some(Workspaces {
            count: 4,
            active: 2
        })
    );
    assert_eq!(world.focused_output(), Some(OutputId(1)));
    assert_eq!(focused(&world), Some(11));
    assert!(placement(&world, 11).visible);
    assert!(placement(&world, 12).visible);
    assert!(!placement(&world, 10).visible);
    // The adopted workspaces carry the origin; the adopter's own does not.
    assert_eq!(
        world.workspace_origins(OutputId(1)),
        Some(vec![None, Some(origin), Some(origin), None])
    );
}

#[test]
fn evicting_with_focus_elsewhere_changes_nothing_on_the_adopter() {
    let mut world = two_workspace_world();
    // Look at the adopter's own window: the standby shape, where the user
    // works on the panel while the other monitor drops.
    world.handle_action(Action::FocusWindowId(WindowId(1)));
    assert_eq!(focused(&world), Some(1));
    let before = world.workspaces(OutputId(1));
    let evicted = evict_second(&mut world);
    assert_eq!(evicted.adopted_by, Some(OutputId(1)));
    assert!(evicted.origin.is_some());
    // No switch: the adopted workspaces join after the adopter's own, and
    // the adopter still shows its own workspace, focus untouched.
    assert_eq!(
        world.workspaces(OutputId(1)),
        Some(Workspaces {
            count: before.expect("output 1 exists").count + 2,
            active: 0
        })
    );
    assert_eq!(world.focused_output(), Some(OutputId(1)));
    assert_eq!(focused(&world), Some(1));
    assert!(placement(&world, 1).visible);
    assert!(!placement(&world, 11).visible);
    // ...and there is no previous view to return to on restore.
    assert_eq!(evicted.adopter_active, None);
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
    // The adopter keeps only its own window, and goes back to the view it
    // had before the adoption (workspace 1, window 1).
    assert_eq!(placement(&world, 1).output, OutputId(1));
    assert_eq!(
        world.workspaces(OutputId(1)),
        Some(Workspaces {
            count: 2,
            active: 0
        })
    );
    // Focus follows the carried window home: window 11 was focused across
    // the eviction, so the returned monitor takes focus with it.
    assert_eq!(world.focused_output(), Some(OutputId(3)));
    assert_eq!(focused(&world), Some(11));
}

#[test]
fn restoring_without_a_carried_focus_leaves_focus_alone() {
    let mut world = two_workspace_world();
    // Focus stays on the adopter's own window across the eviction (the
    // standby shape): nothing switches, so nothing follows home either.
    world.handle_action(Action::FocusWindowId(WindowId(1)));
    let evicted = evict_second(&mut world);
    add_output(&mut world, 3, SECOND);
    world.restore_output(OutputId(3), evicted);

    for id in [10, 11, 12, 13] {
        assert_eq!(placement(&world, id).output, OutputId(3), "window {id}");
    }
    // The adopter's view never moved, and neither did focus.
    assert_eq!(
        world.workspaces(OutputId(1)),
        Some(Workspaces {
            count: 2,
            active: 0
        })
    );
    assert_eq!(world.focused_output(), Some(OutputId(1)));
    assert_eq!(focused(&world), Some(1));
}

/// The review's scratch repro for the switch breaking restore: the adopter
/// sits on an empty, non-trailing workspace, so the switch's normalize drops
/// it and the adopted block shifts down one. `adopted_at` is recorded after
/// that normalize, so the restore still finds every window.
#[test]
fn switching_from_an_empty_active_workspace_still_restores() {
    let mut world = world();
    open(&mut world, 1);
    world.handle_action(Action::FocusWorkspace(Vertical::Down));
    open(&mut world, 2);
    world.handle_action(Action::FocusWorkspace(Vertical::Down));
    open(&mut world, 3);
    world.handle_action(Action::FocusWorkspace(Vertical::Up));
    close(&mut world, 2);
    assert_eq!(
        world.workspaces(OutputId(1)),
        Some(Workspaces {
            count: 4,
            active: 1
        }),
        "adopter sits on an empty, non-trailing workspace"
    );
    add_output(&mut world, 2, SECOND);
    world.handle_action(Action::FocusOutput(OutputId(2)));
    open_on(&mut world, 10, 2, true);

    let evicted = evict_second(&mut world);
    // The switch dropped the empty view workspace: the block moved from 3
    // to 2, and there is no previous view to return to.
    assert_eq!(evicted.adopted_at, 2);
    assert_eq!(evicted.adopter_active, None);
    assert_eq!(
        world.workspaces(OutputId(1)),
        Some(Workspaces {
            count: 4,
            active: 2
        })
    );
    assert_eq!(focused(&world), Some(10));

    add_output(&mut world, 3, SECOND);
    let moved = world.restore_output(OutputId(3), evicted);
    assert_eq!(moved, 1);
    assert_eq!(placement(&world, 10).output, OutputId(3));
    assert_eq!(world.focused_output(), Some(OutputId(3)));
}

/// A trailing sitter goes back to the trailing empty workspace: the adopter
/// was looking at nothing, and the restore returns it to nothing.
#[test]
fn restoring_returns_a_trailing_sitter_to_the_trailing_empty() {
    let mut world = world();
    open(&mut world, 1);
    world.handle_action(Action::FocusWorkspace(Vertical::Down));
    add_output(&mut world, 2, SECOND);
    world.handle_action(Action::FocusOutput(OutputId(2)));
    open_on(&mut world, 10, 2, true);

    let evicted = evict_second(&mut world);
    assert_eq!(evicted.adopter_active, Some(AdopterView::Trailing));
    assert_eq!(
        world.workspaces(OutputId(1)),
        Some(Workspaces {
            count: 3,
            active: 1
        })
    );

    add_output(&mut world, 3, SECOND);
    world.restore_output(OutputId(3), evicted);
    assert_eq!(placement(&world, 10).output, OutputId(3));
    assert_eq!(
        world.workspaces(OutputId(1)),
        Some(Workspaces {
            count: 2,
            active: 1
        }),
        "the adopter is back on its trailing empty workspace"
    );
}

/// A chained unplug keeps the earliest origin: workspaces adopted twice are
/// still named for where they first came from.
#[test]
fn a_chained_unplug_keeps_the_earliest_origin() {
    let mut world = world();
    open(&mut world, 1);
    add_output(&mut world, 2, SECOND);
    world.handle_action(Action::FocusOutput(OutputId(2)));
    open_on(&mut world, 10, 2, true);
    add_output(&mut world, 3, SECOND);

    world.handle_action(Action::FocusOutput(OutputId(1)));
    let first = world.evict_output(OutputId(2)).expect("output 2 exists");
    let first_origin = first.origin.expect("a non-empty adoption is tagged");
    world.handle_action(Action::FocusOutput(OutputId(1)));
    let second = world.evict_output(OutputId(1)).expect("output 1 exists");
    let second_origin = second.origin.expect("a non-empty adoption is tagged");
    assert_ne!(first_origin, second_origin);

    // Output 3 holds output 1's own workspace (tagged with the second
    // origin) and output 2's workspace (still tagged with the first).
    let origins = world
        .workspace_origins(OutputId(3))
        .expect("output 3 exists");
    assert!(origins.contains(&Some(first_origin)));
    assert!(origins.contains(&Some(second_origin)));
    assert_eq!(
        world.window_workspace(WindowId(10)),
        Some((OutputId(3), 1, Some(first_origin)))
    );
    assert_eq!(
        world.window_workspace(WindowId(1)),
        Some((OutputId(3), 0, Some(second_origin)))
    );
}

/// A partial restore untags what stays: windows the user kept on the adopter
/// no longer claim the monitor that came back.
#[test]
fn a_partial_restore_untags_what_stays() {
    let mut world = two_workspace_world();
    let evicted = evict_second(&mut world);
    let origin = evicted.origin.expect("a non-empty adoption is tagged");
    // A new window on the adopted workspace: not in the snapshot, so it
    // stays where it is.
    open(&mut world, 20);
    assert_eq!(placement(&world, 20).output, OutputId(1));

    add_output(&mut world, 3, SECOND);
    let moved = world.restore_output(OutputId(3), evicted);
    assert_eq!(moved, 4);
    assert_eq!(placement(&world, 20).output, OutputId(1));
    for id in [10, 11, 12, 13] {
        assert_eq!(placement(&world, id).output, OutputId(3), "window {id}");
    }
    assert!(
        world
            .workspace_origins(OutputId(1))
            .expect("output 1 exists")
            .iter()
            .all(|o| o != &Some(origin)),
        "nothing on the adopter still names the returned monitor"
    );
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

/// Returning to the pre-adopt view drops the emptied adopted workspace left
/// behind: the restore's first normalize can only keep it while it is
/// active, and the return moves the active workspace elsewhere. Without the
/// second normalize it strands a stray empty workspace (found by the
/// randomized invariant test).
#[test]
fn returning_leaves_no_emptied_adopted_workspace_behind() {
    let mut world = two_workspace_world();
    let evicted = evict_second(&mut world);
    // A new window, carried onto the adopter's first adopted workspace by
    // hand, then back to the second: the active adopted workspace will
    // empty out entirely at restore while its neighbour keeps a window.
    open(&mut world, 20);
    world.handle_action(Action::MoveWindowToWorkspaceIndex(1));
    world.handle_action(Action::FocusWorkspaceIndex(2));
    // Carrying window 20 out left focus on window 12, which the restore
    // then carries home.
    assert_eq!(focused(&world), Some(12));

    add_output(&mut world, 3, SECOND);
    let moved = world.restore_output(OutputId(3), evicted);
    assert_eq!(moved, 4);
    assert_eq!(placement(&world, 20).output, OutputId(1));
    for id in [10, 11, 12, 13] {
        assert_eq!(placement(&world, id).output, OutputId(3), "window {id}");
    }
    assert_eq!(
        world.workspaces(OutputId(1)),
        Some(Workspaces {
            count: 3,
            active: 0
        }),
        "the adopter is back on its own workspace with no strays"
    );
    assert_eq!(world.focused_output(), Some(OutputId(3)));
    assert_eq!(focused(&world), Some(12));
}
