//! Fullscreen: what it covers, what it hides, what ends it, and that leaving
//! puts the layout back exactly.
//!
//! The screen here is the shared 1000x600 with a 10px gap. Several tests add
//! a 30px bar reservation across the top on purpose: a fullscreen window must
//! cover the output's whole *area*, and the gap-inset usable area is exactly
//! where strip arithmetic would put it instead.

use super::*;
use crate::{Action, Horizontal, Size, SizeHints, Vertical};

const BAR: Rect = Rect::new(0, 30, 1000, 570);
const SECOND: Rect = Rect::new(1000, 0, 800, 600);

fn toggle(world: &mut World) {
    world.handle_action(Action::ToggleFullscreen);
}

fn request(world: &mut World, id: u64, fullscreen: bool) {
    world.handle_event(Event::FullscreenRequested {
        id: WindowId(id),
        fullscreen,
    });
}

fn covering(world: &World, output: u64) -> Option<u64> {
    world.fullscreen_on(OutputId(output)).map(|id| id.0)
}

fn with_bar(world: &mut World) {
    world.handle_event(Event::OutputUsableAreaChanged {
        id: OutputId(1),
        area: BAR,
    });
}

/// Asserts `id` covers output 1 edge to edge, and nothing else on it shows.
fn assert_covers(world: &World, id: u64) {
    assert_eq!(covering(world, 1), Some(id));
    let arrangement = world.arrange();
    for placed in &arrangement.placements {
        if placed.id == WindowId(id) {
            assert!(placed.fullscreen, "{placed:?}");
            assert!(placed.visible, "{placed:?}");
            assert_eq!(
                placed.rect, SCREEN,
                "the whole output, bar and gaps included"
            );
        } else if placed.output == OutputId(1) {
            assert!(!placed.visible, "{placed:?} shows beside a covering window");
        }
    }
}

#[test]
fn entering_covers_the_whole_output_wherever_the_column_is() {
    // First, middle and last column, each with a bar reserving the top.
    for target in 1..=3 {
        let mut world = world();
        with_bar(&mut world);
        for id in 1..=3 {
            open(&mut world, id);
        }
        world.handle_action(Action::FocusWindowId(WindowId(target)));
        toggle(&mut world);
        assert!(world.is_fullscreen(WindowId(target)));
        assert_covers(&world, target);
    }
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
        assert!(!world.is_fullscreen(WindowId(target)));
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
    assert_covers(&world, 2);
    request(&mut world, 2, true);
    assert_covers(&world, 2);
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
    world.handle_action(Action::SetFullscreen {
        id: WindowId(u64::MAX),
        fullscreen: true,
    });
    assert_eq!(world.arrange(), before);
    assert!(!world.is_fullscreen(WindowId(99)));
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
fn set_fullscreen_targets_a_window_by_id_without_moving_focus() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::SetFullscreen {
        id: WindowId(1),
        fullscreen: true,
    });
    assert!(world.is_fullscreen(WindowId(1)));
    // Not focused, so not covering: focus stays on 2, and the screen with it.
    assert_eq!(focused(&world), Some(2));
    assert_eq!(covering(&world, 1), None);
    assert!(placement(&world, 2).visible);
    world.handle_action(Action::FocusColumn(Horizontal::Left));
    assert_covers(&world, 1);
}

#[test]
fn focusing_away_scrolls_to_the_neighbour_and_back_covers_again() {
    let mut world = world();
    with_bar(&mut world);
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::FocusColumn(Horizontal::Left));
    toggle(&mut world);
    assert_covers(&world, 1);

    world.handle_action(Action::FocusColumn(Horizontal::Right));
    assert_eq!(covering(&world, 1), None);
    assert!(
        world.is_fullscreen(WindowId(1)),
        "focus moving away is not leaving"
    );
    let neighbour = placement(&world, 2);
    assert!(neighbour.visible);
    // Laid out the ordinary way, within the usable area.
    assert_eq!(neighbour.rect.y, BAR.y + 10);
    // The fullscreen window keeps its size, one ordinary gap to the left.
    let beside = placement(&world, 1);
    assert_eq!(beside.rect.size(), SCREEN.size());
    assert_eq!(beside.rect.right() + 10, neighbour.rect.x);

    world.handle_action(Action::FocusColumn(Horizontal::Left));
    assert_covers(&world, 1);
}

#[test]
fn switching_workspaces_and_back_shows_it_fullscreen_again() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::MoveWindowToWorkspace(Vertical::Down));
    toggle(&mut world);
    assert_covers(&world, 2);
    world.handle_action(Action::FocusWorkspace(Vertical::Up));
    assert_eq!(covering(&world, 1), None);
    assert!(!placement(&world, 2).visible);
    assert!(placement(&world, 1).visible);
    world.handle_action(Action::FocusWorkspace(Vertical::Down));
    assert_covers(&world, 2);
}

#[test]
fn a_window_closing_while_fullscreen_just_leaves_the_layout() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    toggle(&mut world);
    assert_covers(&world, 2);
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
    assert_covers(&world, 2);
    let sibling = placement(&world, 1);
    assert!(!sibling.visible && !sibling.fullscreen);

    world.handle_action(Action::FocusWindow(Vertical::Up));
    assert_eq!(focused(&world), Some(1));
    assert!(!world.is_fullscreen(WindowId(2)));
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
        assert!(!world.is_fullscreen(WindowId(2)), "observed: {observed}");
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
    assert!(!world.is_fullscreen(WindowId(1)));
    assert_eq!(world.arrange(), before);
}

#[test]
fn moving_the_fullscreen_window_within_its_column_keeps_it() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    toggle(&mut world);
    world.handle_action(Action::MoveWindow(Vertical::Up));
    assert_covers(&world, 2);
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
    assert_covers(&world, 3);
    world.handle_action(Action::MoveColumn(Horizontal::Left));
    assert_covers(&world, 3);
}

#[test]
fn consume_and_expel_end_it() {
    // Consume: 2 joins 1's column.
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    toggle(&mut world);
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    assert!(!world.is_fullscreen(WindowId(2)));
    assert_eq!(covering(&world, 1), None);

    // Expel: 2 leaves the column it shares with 1.
    let mut world = super::world();
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    toggle(&mut world);
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Right));
    assert!(!world.is_fullscreen(WindowId(2)));
    assert!(placement(&world, 2).visible);
    assert_eq!(placement(&world, 2).rect.w, 485);
}

#[test]
fn consuming_another_window_into_a_fullscreen_column_ends_it() {
    let mut world = world();
    open(&mut world, 1);
    toggle(&mut world);
    open(&mut world, 2);
    // 2 opened right of 1 and took focus; now it joins 1's column, taking
    // that column's focus from the fullscreen window.
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    assert_eq!(focused(&world), Some(2));
    assert!(!world.is_fullscreen(WindowId(1)));
    assert!(placement(&world, 1).visible && placement(&world, 2).visible);
}

#[test]
fn an_ignored_consume_changes_nothing_fullscreen_included() {
    let mut world = world();
    open(&mut world, 1);
    toggle(&mut world);
    let before = world.arrange();
    // Alone at the strip's edge: nothing to consume into.
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Right));
    assert_eq!(world.arrange(), before);
    assert_covers(&world, 1);
}

#[test]
fn moving_to_another_workspace_or_output_ends_it() {
    let mut world = world();
    open(&mut world, 1);
    toggle(&mut world);
    world.handle_action(Action::MoveWindowToWorkspace(Vertical::Down));
    assert!(!world.is_fullscreen(WindowId(1)));
    assert_eq!(placement(&world, 1).rect, Rect::new(10, 10, 485, 580));

    let mut world = super::world();
    open(&mut world, 1);
    open(&mut world, 2);
    toggle(&mut world);
    world.handle_action(Action::MoveWindowToWorkspaceIndex(1));
    assert!(!world.is_fullscreen(WindowId(2)));

    let mut world = super::world();
    world.handle_event(Event::OutputAdded {
        id: OutputId(2),
        area: SECOND,
    });
    open(&mut world, 1);
    toggle(&mut world);
    world.handle_action(Action::MoveFocusedWindowToOutput(OutputId(2)));
    assert!(!world.is_fullscreen(WindowId(1)));
    assert_eq!(placement(&world, 1).output, OutputId(2));
    assert_eq!(covering(&world, 1), None);
    assert_eq!(covering(&world, 2), None);
}

#[test]
fn ignored_moves_change_nothing_fullscreen_included() {
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
    assert_covers(&world, 1);
}

#[test]
fn each_output_has_its_own_fullscreen() {
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
    assert_eq!(placed.rect, SECOND);
    assert!(placed.visible);
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
fn it_follows_its_output_s_area_and_ignores_reservations() {
    let mut world = world();
    open(&mut world, 1);
    toggle(&mut world);
    let bigger = Rect::new(0, 0, 1600, 900);
    world.handle_event(Event::OutputChanged {
        id: OutputId(1),
        area: bigger,
    });
    assert_eq!(placement(&world, 1).rect, bigger);
    world.handle_event(Event::OutputUsableAreaChanged {
        id: OutputId(1),
        area: Rect::new(0, 40, 1600, 860),
    });
    assert_eq!(placement(&world, 1).rect, bigger);
}

#[test]
fn an_adopted_fullscreen_window_covers_its_new_output_when_focused() {
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
    assert!(world.is_fullscreen(WindowId(2)));
    world.handle_action(Action::FocusWindowId(WindowId(2)));
    assert_covers(&world, 2);
}

#[test]
fn a_window_that_asks_before_there_is_an_output_is_fullscreen_once_placed() {
    let mut world = World::new(config());
    open(&mut world, 1);
    request(&mut world, 1, true);
    assert!(world.is_fullscreen(WindowId(1)));
    world.handle_event(Event::OutputAdded {
        id: OutputId(1),
        area: SCREEN,
    });
    assert_covers(&world, 1);
}

#[test]
fn a_fullscreen_frame_teaches_no_minimum() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    toggle(&mut world);
    // Sized for the whole output, larger than any column: while fullscreen
    // this is the size it was asked for, not a refusal to shrink.
    world.handle_event(Event::FrameObserved {
        id: WindowId(2),
        requested: Size::new(485, 580),
        actual: SCREEN.size(),
    });
    toggle(&mut world);
    assert_eq!(placement(&world, 2).rect.w, 485);
}

#[test]
fn a_minimum_width_does_not_stop_it_covering() {
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
    assert_covers(&world, 1);
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
    assert_covers(&world, 2);
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

/// Focused away, a fullscreen column sits in the strip exactly where a tiled
/// column of the output's width would: one ordinary gap from each neighbour,
/// never over the focused window -- with and without a reserved left edge,
/// whose width a placement measured from `area.x` would overlap by.
#[test]
fn focused_away_it_keeps_ordinary_gaps_on_both_sides() {
    for usable in [SCREEN, Rect::new(40, 0, 960, 600), BAR] {
        // Left neighbour: 1 is focused, 2 (right of it) is fullscreen.
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
        let full = placement(&world, 2);
        assert_eq!(focused.rect.x, usable.x + 10, "{usable:?}");
        assert_eq!(
            full.rect.x,
            focused.rect.right() + 10,
            "left neighbour, {usable:?}: {full:?}"
        );

        // Right neighbour: 2 is focused, 1 (left of it) is fullscreen.
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
        let full = placement(&world, 1);
        assert_eq!(
            full.rect.right() + 10,
            focused.rect.x,
            "right neighbour, {usable:?}: {full:?}"
        );
    }
}
