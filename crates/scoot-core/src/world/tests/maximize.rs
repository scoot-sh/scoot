//! Maximized: what it fills, what it hides, what ends it, and that leaving
//! puts the layout back exactly.
//!
//! The screen here is the shared 1000x600 with a 10px gap, so the usable
//! area with no bar is (10, 10, 980, 580). Several tests add a 30px bar
//! reservation across the top on purpose: a maximized window must fill the
//! *usable* area -- (10, 40, 980, 550) -- where a fullscreen window would
//! cover the whole output instead.

use super::*;
use crate::{Action, Horizontal, Size, SizeHints, Vertical};

const BAR: Rect = Rect::new(0, 30, 1000, 570);
/// The usable area with the bar above, minus the gap: what a maximized
/// window fills while it covers.
const BAR_USABLE: Rect = Rect::new(10, 40, 980, 550);
/// The usable area with no bar, minus the gap.
const NO_BAR_USABLE: Rect = Rect::new(10, 10, 980, 580);
const SECOND: Rect = Rect::new(1000, 0, 800, 600);

fn toggle(world: &mut World) {
    world.handle_action(Action::ToggleMaximize);
}

fn request(world: &mut World, id: u64, maximized: bool) {
    world.handle_event(Event::MaximizeRequested {
        id: WindowId(id),
        maximized,
    });
}

fn covering(world: &World, output: u64) -> Option<u64> {
    world.maximized_on(OutputId(output)).map(|id| id.0)
}

fn with_bar(world: &mut World) {
    world.handle_event(Event::OutputUsableAreaChanged {
        id: OutputId(1),
        area: BAR,
    });
}

/// Asserts `id` fills output 1's usable area, and nothing else tiled on it
/// shows. (Floating windows stay above a tiled maximized window.)
fn assert_covers(world: &World, id: u64, usable: Rect) {
    assert_eq!(covering(world, 1), Some(id));
    let arrangement = world.arrange();
    for placed in &arrangement.placements {
        if placed.id == WindowId(id) {
            assert!(placed.maximized, "{placed:?}");
            assert!(!placed.fullscreen, "{placed:?}");
            assert!(placed.visible, "{placed:?}");
            assert_eq!(
                placed.rect, usable,
                "the usable area, gaps kept and bar excluded"
            );
            assert_eq!(
                placed.requested,
                Some(usable.size()),
                "the client is asked for the usable size"
            );
        } else if placed.output == OutputId(1) && !placed.floating {
            assert!(!placed.visible, "{placed:?} shows beside a covering window");
        }
    }
}

#[test]
fn entering_fills_the_usable_area_wherever_the_column_is() {
    // First, middle and last column, each with a bar reserving the top.
    for target in 1..=3 {
        let mut world = world();
        with_bar(&mut world);
        for id in 1..=3 {
            open(&mut world, id);
        }
        world.handle_action(Action::FocusWindowId(WindowId(target)));
        toggle(&mut world);
        assert!(world.is_maximized(WindowId(target)));
        assert_covers(&world, target, BAR_USABLE);
    }
}

#[test]
fn entering_with_no_bar_fills_the_gap_inset_area() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    toggle(&mut world);
    assert_covers(&world, 2, NO_BAR_USABLE);
}

#[test]
fn leaving_restores_the_whole_arrangement_exactly() {
    for target in 1..=4 {
        let mut world = world();
        with_bar(&mut world);
        for id in 1..=4 {
            open(&mut world, id);
        }
        // Scroll to a point where the target sits on the right of the view,
        // so entering (which lines the view up with its left edge) moves it.
        world.handle_action(Action::FocusWindowId(WindowId(target)));
        let before = world.arrange();
        toggle(&mut world);
        assert_ne!(world.arrange(), before);
        toggle(&mut world);
        assert!(!world.is_maximized(WindowId(target)));
        assert_eq!(world.arrange(), before, "column {target} did not come back");
    }
}

#[test]
fn the_window_s_own_request_enters_and_leaves_the_same_way() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    let before = world.arrange();
    request(&mut world, 2, true);
    assert_covers(&world, 2, NO_BAR_USABLE);
    request(&mut world, 2, true);
    assert_covers(&world, 2, NO_BAR_USABLE);
    request(&mut world, 2, false);
    assert_eq!(world.arrange(), before);
    // Leaving twice changes nothing either.
    request(&mut world, 2, false);
    assert_eq!(world.arrange(), before);
}

#[test]
fn unknown_windows_are_ignored() {
    let mut world = world();
    open(&mut world, 1);
    let before = world.arrange();
    request(&mut world, 99, true);
    world.handle_action(Action::SetMaximized {
        id: WindowId(u64::MAX),
        maximized: true,
    });
    assert_eq!(world.arrange(), before);
    assert!(!world.is_maximized(WindowId(99)));
    assert_eq!(covering(&world, 1), None);
    assert_eq!(covering(&world, 42), None, "an unknown output");
}

#[test]
fn toggling_with_nothing_focused_does_nothing() {
    let mut world = world();
    toggle(&mut world);
    assert!(world.arrange().placements.is_empty());
    assert_eq!(covering(&world, 1), None);
}

#[test]
fn set_maximized_targets_a_window_by_id_without_moving_focus() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::SetMaximized {
        id: WindowId(1),
        maximized: true,
    });
    assert!(world.is_maximized(WindowId(1)));
    // Not focused, so not covering: focus stays on 2, and the screen with it.
    assert_eq!(focused(&world), Some(2));
    assert_eq!(covering(&world, 1), None);
    assert!(placement(&world, 2).visible);
    assert!(!placement(&world, 2).maximized);
    world.handle_action(Action::FocusColumn(Horizontal::Left));
    assert_covers(&world, 1, NO_BAR_USABLE);
}

#[test]
fn set_maximized_is_idempotent_and_keeps_focus() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    let before_focus = focused(&world);
    world.handle_action(Action::SetMaximized {
        id: WindowId(2),
        maximized: true,
    });
    let covered = world.arrange();
    world.handle_action(Action::SetMaximized {
        id: WindowId(2),
        maximized: true,
    });
    assert_eq!(world.arrange(), covered);
    assert_eq!(focused(&world), before_focus);
}

#[test]
fn focusing_away_keeps_usable_size_and_back_covers_again() {
    let mut world = world();
    with_bar(&mut world);
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::FocusColumn(Horizontal::Left));
    toggle(&mut world);
    assert_covers(&world, 1, BAR_USABLE);

    world.handle_action(Action::FocusColumn(Horizontal::Right));
    assert_eq!(covering(&world, 1), None);
    assert!(
        world.is_maximized(WindowId(1)),
        "focus moving away is not leaving"
    );
    let neighbour = placement(&world, 2);
    assert!(neighbour.visible);
    // Laid out the ordinary way, within the usable area.
    assert_eq!(neighbour.rect.y, BAR.y + 10);
    // The maximized window keeps its usable size, one ordinary gap to the
    // left of the focused window -- and never over it.
    let beside = placement(&world, 1);
    assert!(beside.maximized);
    assert_eq!(beside.rect.size(), BAR_USABLE.size());
    assert_eq!(beside.rect.right() + 10, neighbour.rect.x);

    world.handle_action(Action::FocusColumn(Horizontal::Left));
    assert_covers(&world, 1, BAR_USABLE);
}

#[test]
fn switching_workspaces_and_back_shows_it_maximized_again() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::MoveWindowToWorkspace(Vertical::Down));
    toggle(&mut world);
    assert_covers(&world, 2, NO_BAR_USABLE);
    world.handle_action(Action::FocusWorkspace(Vertical::Up));
    assert_eq!(covering(&world, 1), None);
    assert!(!placement(&world, 2).visible);
    assert!(placement(&world, 1).visible);
    world.handle_action(Action::FocusWorkspace(Vertical::Down));
    assert_covers(&world, 2, NO_BAR_USABLE);
}

#[test]
fn a_window_closing_while_maximized_just_leaves_the_layout() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    toggle(&mut world);
    assert_covers(&world, 2, NO_BAR_USABLE);
    world.handle_event(Event::WindowClosed { id: WindowId(2) });
    assert_eq!(covering(&world, 1), None);
    assert_eq!(focused(&world), Some(1));
    assert_eq!(placement(&world, 1).rect, Rect::new(10, 10, 485, 580));
    assert!(placement(&world, 1).visible);
}

#[test]
fn stacked_siblings_are_hidden_and_focusing_one_ends_it() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    // Column [1, 2], 2 focused.
    toggle(&mut world);
    assert_covers(&world, 2, NO_BAR_USABLE);
    let sibling = placement(&world, 1);
    assert!(!sibling.visible && !sibling.maximized);

    world.handle_action(Action::FocusWindow(Vertical::Up));
    assert_eq!(focused(&world), Some(1));
    assert!(!world.is_maximized(WindowId(2)));
    assert_eq!(covering(&world, 1), None);
    assert!(placement(&world, 1).visible && placement(&world, 2).visible);
}

#[test]
fn focusing_a_hidden_sibling_by_id_or_by_observation_ends_it() {
    for observed in [false, true] {
        let mut world = world();
        open(&mut world, 1);
        open(&mut world, 2);
        world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
        toggle(&mut world);
        if observed {
            world.handle_event(Event::FocusObserved { id: WindowId(1) });
        } else {
            world.handle_action(Action::FocusWindowId(WindowId(1)));
        }
        assert!(!world.is_maximized(WindowId(2)), "observed: {observed}");
        assert!(placement(&world, 1).visible, "observed: {observed}");
    }
}

#[test]
fn a_window_that_is_not_its_column_s_focused_one_cannot_enter() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    let before = world.arrange();
    request(&mut world, 1, true);
    assert!(!world.is_maximized(WindowId(1)));
    assert_eq!(world.arrange(), before);
}

#[test]
fn moving_the_maximized_window_within_its_column_keeps_it() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    toggle(&mut world);
    world.handle_action(Action::MoveWindow(Vertical::Up));
    assert_covers(&world, 2, NO_BAR_USABLE);
}

#[test]
fn moving_its_column_keeps_it_covering() {
    let mut world = world();
    with_bar(&mut world);
    for id in 1..=3 {
        open(&mut world, id);
    }
    toggle(&mut world);
    world.handle_action(Action::MoveColumn(Horizontal::Left));
    assert_covers(&world, 3, BAR_USABLE);
    world.handle_action(Action::MoveColumn(Horizontal::Left));
    assert_covers(&world, 3, BAR_USABLE);
}

#[test]
fn consume_and_expel_end_it() {
    // Consume: 2 joins 1's column.
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    toggle(&mut world);
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    assert!(!world.is_maximized(WindowId(2)));
    assert_eq!(covering(&world, 1), None);

    // Expel: 2 leaves the column it shares with 1.
    let mut world = super::world();
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    toggle(&mut world);
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Right));
    assert!(!world.is_maximized(WindowId(2)));
    assert!(placement(&world, 2).visible);
    assert_eq!(placement(&world, 2).rect.w, 485);
}

#[test]
fn consuming_another_window_into_a_maximized_column_ends_it() {
    let mut world = world();
    open(&mut world, 1);
    toggle(&mut world);
    open(&mut world, 2);
    // 2 opened right of 1 and took focus; now it joins 1's column, taking
    // that column's focus from the maximized window.
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    assert_eq!(focused(&world), Some(2));
    assert!(!world.is_maximized(WindowId(1)));
    assert!(placement(&world, 1).visible && placement(&world, 2).visible);
}

#[test]
fn an_ignored_consume_changes_nothing_maximized_included() {
    let mut world = world();
    open(&mut world, 1);
    toggle(&mut world);
    let before = world.arrange();
    // Alone at the strip's edge: nothing to consume into.
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Right));
    assert_eq!(world.arrange(), before);
    assert_covers(&world, 1, NO_BAR_USABLE);
}

#[test]
fn moving_to_another_workspace_or_output_ends_it() {
    let mut world = world();
    open(&mut world, 1);
    toggle(&mut world);
    world.handle_action(Action::MoveWindowToWorkspace(Vertical::Down));
    assert!(!world.is_maximized(WindowId(1)));
    assert_eq!(placement(&world, 1).rect, Rect::new(10, 10, 485, 580));

    let mut world = super::world();
    open(&mut world, 1);
    open(&mut world, 2);
    toggle(&mut world);
    world.handle_action(Action::MoveWindowToWorkspaceIndex(1));
    assert!(!world.is_maximized(WindowId(2)));

    let mut world = super::world();
    world.handle_event(Event::OutputAdded {
        id: OutputId(2),
        area: SECOND,
    });
    open(&mut world, 1);
    toggle(&mut world);
    world.handle_action(Action::MoveFocusedWindowToOutput(OutputId(2)));
    assert!(!world.is_maximized(WindowId(1)));
    assert_eq!(placement(&world, 1).output, OutputId(2));
    assert_eq!(covering(&world, 1), None);
    assert_eq!(covering(&world, 2), None);
}

#[test]
fn ignored_moves_change_nothing_maximized_included() {
    let mut world = world();
    open(&mut world, 1);
    toggle(&mut world);
    let before = world.arrange();
    world.handle_action(Action::MoveWindowToWorkspace(Vertical::Up));
    world.handle_action(Action::MoveWindowToWorkspaceIndex(0));
    world.handle_action(Action::MoveWindowToWorkspaceIndex(usize::MAX));
    world.handle_action(Action::MoveFocusedWindowToOutput(OutputId(1)));
    world.handle_action(Action::MoveFocusedWindowToOutput(OutputId(999)));
    assert_eq!(world.arrange(), before);
    assert_covers(&world, 1, NO_BAR_USABLE);
}

#[test]
fn each_output_has_its_own_maximized() {
    let mut world = world();
    world.handle_event(Event::OutputAdded {
        id: OutputId(2),
        area: SECOND,
    });
    open(&mut world, 1);
    world.handle_event(Event::WindowOpened {
        id: WindowId(2),
        info: WindowInfo::default(),
        output: Some(OutputId(2)),
        focus: true,
    });
    toggle(&mut world);
    assert_eq!(covering(&world, 2), Some(2));
    assert_eq!(covering(&world, 1), None);
    let placed = placement(&world, 2);
    assert_eq!(placed.rect, Rect::new(1010, 10, 780, 580));
    assert!(placed.visible && placed.maximized);
    // Output 1 is untouched: its window is still laid out and shown.
    assert!(placement(&world, 1).visible);
    assert_eq!(placement(&world, 1).rect, Rect::new(10, 10, 485, 580));
    // Focus moving to the other output does not end or hide it: output 2's
    // active workspace still has it in focus.
    world.handle_action(Action::FocusOutput(OutputId(1)));
    assert_eq!(covering(&world, 2), Some(2));
    assert!(placement(&world, 2).visible);
}

#[test]
fn it_follows_the_usable_area_and_the_bar() {
    let mut world = world();
    open(&mut world, 1);
    toggle(&mut world);
    assert_covers(&world, 1, NO_BAR_USABLE);
    // A bar reserving the top shrinks what it fills; the bar's own strip
    // stays outside.
    with_bar(&mut world);
    assert_covers(&world, 1, BAR_USABLE);
    // An output resize moves and resizes it with the usable area -- once
    // the platform re-reports the reservation against the new size (a
    // geometry change only re-clamps what was reserved, per
    // `Output::set_area`).
    let bigger = Rect::new(0, 0, 1600, 900);
    world.handle_event(Event::OutputChanged {
        id: OutputId(1),
        area: bigger,
    });
    assert_covers(&world, 1, BAR_USABLE);
    world.handle_event(Event::OutputUsableAreaChanged {
        id: OutputId(1),
        area: Rect::new(0, 30, 1600, 870),
    });
    assert_covers(&world, 1, Rect::new(10, 40, 1580, 850));
    // Leaving still restores the plain strip.
    toggle(&mut world);
    assert!(!placement(&world, 1).maximized);
}

#[test]
fn a_bar_on_each_edge_is_respected() {
    // A 30px reservation on each edge in turn (and none): the maximized
    // window fills exactly what is left, minus the gap.
    for (usable, filled) in [
        (BAR, BAR_USABLE),
        (Rect::new(0, 0, 1000, 570), Rect::new(10, 10, 980, 550)),
        (Rect::new(0, 30, 1000, 570), BAR_USABLE),
        (Rect::new(30, 0, 970, 600), Rect::new(40, 10, 950, 580)),
        (Rect::new(0, 0, 970, 600), Rect::new(10, 10, 950, 580)),
        (SCREEN, NO_BAR_USABLE),
    ] {
        let mut world = world();
        world.handle_event(Event::OutputUsableAreaChanged {
            id: OutputId(1),
            area: usable,
        });
        open(&mut world, 1);
        toggle(&mut world);
        assert_covers(&world, 1, filled);
    }
}

#[test]
fn an_adopted_maximized_window_covers_its_new_output_when_focused() {
    let mut world = world();
    world.handle_event(Event::OutputAdded {
        id: OutputId(2),
        area: SECOND,
    });
    open(&mut world, 1);
    world.handle_event(Event::WindowOpened {
        id: WindowId(2),
        info: WindowInfo::default(),
        output: Some(OutputId(2)),
        focus: true,
    });
    toggle(&mut world);
    world.handle_event(Event::OutputRemoved { id: OutputId(2) });
    assert!(world.is_maximized(WindowId(2)));
    world.handle_action(Action::FocusWindowId(WindowId(2)));
    assert_covers(&world, 2, NO_BAR_USABLE);
}

#[test]
fn a_window_that_asks_before_there_is_an_output_is_maximized_once_placed() {
    let mut world = World::new(config());
    open(&mut world, 1);
    request(&mut world, 1, true);
    assert!(world.is_maximized(WindowId(1)));
    world.handle_event(Event::OutputAdded {
        id: OutputId(1),
        area: SCREEN,
    });
    assert_covers(&world, 1, NO_BAR_USABLE);
}

#[test]
fn a_maximized_frame_teaches_no_minimum() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    toggle(&mut world);
    // Sized for the usable area, larger than any column: while maximized
    // this is the size it was asked for, not a refusal to shrink.
    world.handle_event(Event::FrameObserved {
        id: WindowId(2),
        requested: Size::new(485, 580),
        actual: NO_BAR_USABLE.size(),
    });
    toggle(&mut world);
    assert_eq!(placement(&world, 2).rect.w, 485);
}

#[test]
fn a_minimum_size_does_not_stop_it_covering() {
    let mut world = world();
    open_with(
        &mut world,
        1,
        WindowInfo {
            hints: SizeHints {
                min: Size::new(700, 300),
                ..SizeHints::default()
            },
            ..WindowInfo::default()
        },
    );
    open(&mut world, 2);
    world.handle_action(Action::FocusColumn(Horizontal::Left));
    toggle(&mut world);
    assert_covers(&world, 1, NO_BAR_USABLE);
}

#[test]
fn a_config_reload_keeps_it_covering() {
    let mut world = world();
    with_bar(&mut world);
    for id in 1..=3 {
        open(&mut world, id);
    }
    world.handle_action(Action::FocusColumn(Horizontal::Left));
    toggle(&mut world);
    world.set_config(Config {
        gap: 40,
        column_widths: vec![0.25],
        default_column_width: 0,
    });
    assert_covers(&world, 2, Rect::new(40, 70, 920, 490));
}

/// Focused away, a maximized column sits in the strip exactly where a tiled
/// column of the usable width would: one ordinary gap from each neighbour,
/// never over the focused window -- with and without a reserved edge.
#[test]
fn focused_away_it_keeps_ordinary_gaps_on_both_sides() {
    for usable in [SCREEN, Rect::new(40, 0, 960, 600), BAR] {
        // Left neighbour: 1 is focused, 2 (right of it) is maximized.
        let mut world = world();
        world.handle_event(Event::OutputUsableAreaChanged {
            id: OutputId(1),
            area: usable,
        });
        open(&mut world, 1);
        open(&mut world, 2);
        toggle(&mut world);
        world.handle_action(Action::FocusColumn(Horizontal::Left));
        let focused = placement(&world, 1);
        let maxed = placement(&world, 2);
        assert!(maxed.maximized);
        assert_eq!(focused.rect.x, usable.x + 10, "{usable:?}");
        assert_eq!(
            maxed.rect.x,
            focused.rect.right() + 10,
            "left neighbour, {usable:?}: {maxed:?}"
        );

        // Right neighbour: 2 is focused, 1 (left of it) is maximized.
        let mut world = super::world();
        world.handle_event(Event::OutputUsableAreaChanged {
            id: OutputId(1),
            area: usable,
        });
        open(&mut world, 1);
        open(&mut world, 2);
        world.handle_action(Action::FocusColumn(Horizontal::Left));
        toggle(&mut world);
        world.handle_action(Action::FocusColumn(Horizontal::Right));
        let focused = placement(&world, 2);
        let maxed = placement(&world, 1);
        assert_eq!(
            maxed.rect.right() + 10,
            focused.rect.x,
            "right neighbour, {usable:?}: {maxed:?}"
        );
    }
}

#[test]
fn width_actions_are_ignored_while_maximized() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    toggle(&mut world);
    assert_covers(&world, 2, NO_BAR_USABLE);
    let before = world.arrange();
    world.handle_action(Action::CycleColumnWidth);
    world.handle_action(Action::SetColumnWidth(1));
    world.handle_action(Action::SetColumnWidth(usize::MAX));
    assert_eq!(
        world.arrange(),
        before,
        "width actions moved a maximized window"
    );
    assert!(world.is_maximized(WindowId(2)));
    // Leaving still restores the width it entered with.
    toggle(&mut world);
    assert_eq!(placement(&world, 2).rect.w, 485);
}

#[test]
fn width_actions_apply_again_after_leaving() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    toggle(&mut world);
    toggle(&mut world);
    world.handle_action(Action::CycleColumnWidth);
    assert_eq!(placement(&world, 2).rect.w, 980);
}

#[test]
fn fullscreen_wins_while_set_and_leaving_it_returns_to_maximized() {
    let mut world = world();
    with_bar(&mut world);
    open(&mut world, 1);
    open(&mut world, 2);
    toggle(&mut world);
    assert_covers(&world, 2, BAR_USABLE);
    // Fullscreen on top: edge to edge, bar included.
    world.handle_action(Action::ToggleFullscreen);
    assert!(world.is_fullscreen(WindowId(2)));
    assert!(world.is_maximized(WindowId(2)));
    assert_eq!(world.fullscreen_on(OutputId(1)), Some(WindowId(2)));
    assert_eq!(covering(&world, 1), None, "fullscreen wins over maximized");
    let placed = placement(&world, 2);
    assert!(placed.fullscreen && !placed.maximized);
    assert_eq!(placed.rect, SCREEN);
    // Leaving fullscreen returns to maximized, not to the plain strip.
    world.handle_action(Action::ToggleFullscreen);
    assert!(!world.is_fullscreen(WindowId(2)));
    assert_covers(&world, 2, BAR_USABLE);
    // And leaving maximized then restores the strip exactly.
    let plain = {
        let mut plain = super::world();
        with_bar(&mut plain);
        open(&mut plain, 1);
        open(&mut plain, 2);
        plain.arrange()
    };
    toggle(&mut world);
    assert_eq!(world.arrange(), plain);
}

#[test]
fn maximized_then_fullscreen_by_request_compose_the_same_way() {
    let mut world = world();
    open(&mut world, 1);
    request(&mut world, 1, true);
    assert_covers(&world, 1, NO_BAR_USABLE);
    world.handle_event(Event::FullscreenRequested {
        id: WindowId(1),
        fullscreen: true,
    });
    assert_eq!(world.fullscreen_on(OutputId(1)), Some(WindowId(1)));
    assert_eq!(covering(&world, 1), None);
    world.handle_event(Event::FullscreenRequested {
        id: WindowId(1),
        fullscreen: false,
    });
    assert_covers(&world, 1, NO_BAR_USABLE);
}

#[test]
fn unmaximizing_a_fullscreen_window_keeps_it_fullscreen() {
    let mut world = world();
    open(&mut world, 1);
    request(&mut world, 1, true);
    world.handle_action(Action::ToggleFullscreen);
    request(&mut world, 1, false);
    assert!(!world.is_maximized(WindowId(1)));
    assert!(world.is_fullscreen(WindowId(1)));
    assert_eq!(world.fullscreen_on(OutputId(1)), Some(WindowId(1)));
}

#[test]
fn a_floating_window_maximizes_into_the_usable_area() {
    let mut world = world();
    with_bar(&mut world);
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::ToggleFloating);
    // Window 2 floats; focus is on the floating layer. Give it a size so
    // leaving maximized has a floating rect to come back to.
    world.handle_event(Event::FrameObserved {
        id: WindowId(2),
        requested: Size::default(),
        actual: Size::new(200, 100),
    });
    let floating_before = world.floating_geometry(WindowId(2));
    request(&mut world, 2, true);
    assert!(world.is_floating(WindowId(2)));
    assert!(world.is_maximized(WindowId(2)));
    assert_eq!(covering(&world, 1), Some(2));
    let placed = placement(&world, 2);
    assert!(placed.maximized && placed.floating && placed.visible);
    assert_eq!(placed.rect, BAR_USABLE);
    assert_eq!(placed.requested, Some(BAR_USABLE.size()));
    // The strip hides under it.
    assert!(!placement(&world, 1).visible);
    // Leaving maximized puts it back where it floated.
    request(&mut world, 2, false);
    assert!(world.is_floating(WindowId(2)));
    assert_eq!(world.floating_geometry(WindowId(2)), floating_before);
}

#[test]
fn a_dialog_above_a_maximized_floating_window_stays_up() {
    let mut world = world();
    open(&mut world, 1);
    world.handle_action(Action::ToggleFloating);
    request(&mut world, 1, true);
    assert_eq!(covering(&world, 1), Some(1));
    // Its dialog floats above, focused: the maximized window stays in
    // place, full usable size, under the floating layer.
    world.handle_event(Event::WindowOpened {
        id: WindowId(2),
        info: WindowInfo {
            parent: Some(WindowId(1)),
            ..WindowInfo::default()
        },
        output: None,
        focus: true,
    });
    world.handle_event(Event::FloatingRequested {
        id: WindowId(2),
        floating: true,
        size: None,
    });
    world.handle_event(Event::FrameObserved {
        id: WindowId(2),
        requested: Size::default(),
        actual: Size::new(200, 100),
    });
    assert_eq!(covering(&world, 1), None);
    let maxed = placement(&world, 1);
    assert!(maxed.maximized && maxed.visible);
    assert_eq!(maxed.rect, NO_BAR_USABLE);
    assert!(placement(&world, 2).visible);
    // An unrelated floating window still hides below it.
    world.handle_event(Event::WindowOpened {
        id: WindowId(3),
        info: WindowInfo::default(),
        output: None,
        focus: false,
    });
    world.handle_event(Event::FloatingRequested {
        id: WindowId(3),
        floating: true,
        size: None,
    });
    world.handle_event(Event::FrameObserved {
        id: WindowId(3),
        requested: Size::default(),
        actual: Size::new(200, 100),
    });
    assert!(!placement(&world, 3).visible);
}

#[test]
fn floating_or_un_floating_a_maximized_window_ends_its_maximized() {
    let mut world = world();
    open(&mut world, 1);
    toggle(&mut world);
    assert_covers(&world, 1, NO_BAR_USABLE);
    world.handle_action(Action::ToggleFloating);
    assert!(world.is_floating(WindowId(1)));
    assert!(!world.is_maximized(WindowId(1)));
    toggle(&mut world);
    assert!(world.is_maximized(WindowId(1)));
    world.handle_action(Action::ToggleFloating);
    assert!(!world.is_floating(WindowId(1)));
    assert!(!world.is_maximized(WindowId(1)));
}

#[test]
fn moves_and_resizes_ignore_a_maximized_floating_window() {
    let mut world = world();
    open(&mut world, 1);
    world.handle_action(Action::ToggleFloating);
    request(&mut world, 1, true);
    let before = world.arrange();
    world.handle_action(Action::MoveFloating {
        id: WindowId(1),
        x: 0,
        y: 0,
    });
    world.handle_action(Action::ResizeFloating {
        id: WindowId(1),
        size: Size::new(100, 100),
        edges: crate::Edges::BOTTOM_RIGHT,
    });
    assert_eq!(world.arrange(), before);
    assert!(world.floating_geometry(WindowId(1)).is_none());
}

#[test]
fn a_maximized_tiled_window_leaves_floating_windows_above_it() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::FocusWindowId(WindowId(1)));
    world.handle_action(Action::ToggleFloating);
    world.handle_action(Action::FocusWindowId(WindowId(2)));
    world.handle_event(Event::FrameObserved {
        id: WindowId(1),
        requested: Size::default(),
        actual: Size::new(200, 100),
    });
    toggle(&mut world);
    // Window 2 maximizes in the strip; window 1 still floats above it.
    assert_covers(&world, 2, NO_BAR_USABLE);
    assert!(placement(&world, 1).visible);
}

#[test]
fn opening_a_window_while_maximized_focuses_away_without_ending_it() {
    let mut world = world();
    open(&mut world, 1);
    toggle(&mut world);
    assert_covers(&world, 1, NO_BAR_USABLE);
    open(&mut world, 2);
    // The new window takes focus; the maximized one stays maximized in its
    // strip slot, one gap from the focused window.
    assert!(world.is_maximized(WindowId(1)));
    assert_eq!(covering(&world, 1), None);
    assert_eq!(focused(&world), Some(2));
    let maxed = placement(&world, 1);
    assert_eq!(maxed.rect.size(), NO_BAR_USABLE.size());
    // Focusing back covers again.
    world.handle_action(Action::FocusWindowId(WindowId(1)));
    assert_covers(&world, 1, NO_BAR_USABLE);
}

#[test]
fn a_zero_sized_output_still_places_it_with_a_real_size() {
    let mut world = world();
    open(&mut world, 1);
    toggle(&mut world);
    world.handle_event(Event::OutputChanged {
        id: OutputId(1),
        area: Rect::new(0, 0, 0, 0),
    });
    let placed = placement(&world, 1);
    assert!(placed.rect.w >= 1 && placed.rect.h >= 1, "{placed:?}");
}
