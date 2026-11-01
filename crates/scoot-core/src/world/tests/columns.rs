use super::*;
use crate::{Action, Effect, Horizontal, Size, SizeHints, Vertical};

fn min_height(h: i32) -> WindowInfo {
    WindowInfo {
        hints: SizeHints {
            min: Size::new(0, h),
            ..SizeHints::default()
        },
        ..WindowInfo::default()
    }
}

#[test]
fn new_windows_open_to_the_right_and_take_focus() {
    let mut world = world();
    open(&mut world, 1);
    assert_eq!(placement(&world, 1).rect, Rect::new(10, 10, 485, 580));
    open(&mut world, 2);
    assert_eq!(placement(&world, 2).rect, Rect::new(505, 10, 485, 580));
    assert_eq!(focused(&world), Some(2));
}

#[test]
fn windows_opened_without_focus_leave_focus_alone() {
    let mut world = world();
    open(&mut world, 1);
    world.handle_event(Event::WindowOpened {
        id: WindowId(2),
        info: WindowInfo::default(),
        output: None,
        focus: false,
    });
    assert_eq!(focused(&world), Some(1));
    assert_eq!(placement(&world, 2).rect.x, 505);
}

#[test]
fn scrolling_keeps_the_focused_column_in_view() {
    let mut world = world();
    for id in 1..=3 {
        open(&mut world, id);
    }
    assert_eq!(placement(&world, 3).rect.x, 505);
    assert!(!placement(&world, 1).visible);
    assert!(placement(&world, 2).visible);

    world.handle_action(Action::FocusColumn(Horizontal::Left));
    world.handle_action(Action::FocusColumn(Horizontal::Left));
    assert_eq!(focused(&world), Some(1));
    assert_eq!(placement(&world, 1).rect.x, 10);
    assert!(!placement(&world, 3).visible);
}

#[test]
fn closing_focuses_a_neighbour_and_scrolls_back() {
    let mut world = world();
    for id in 1..=3 {
        open(&mut world, id);
    }
    world.handle_event(Event::WindowClosed { id: WindowId(3) });
    assert_eq!(focused(&world), Some(2));
    assert_eq!(placement(&world, 2).rect.x, 505);
    assert!(placement(&world, 1).visible);
}

#[test]
fn closing_a_column_left_of_focus_keeps_focus() {
    let mut world = world();
    for id in 1..=3 {
        open(&mut world, id);
    }
    world.handle_event(Event::WindowClosed { id: WindowId(1) });
    assert_eq!(focused(&world), Some(3));
}

#[test]
fn closing_the_middle_of_a_stack_keeps_the_rest_stacked() {
    let mut world = world();
    for id in 1..=3 {
        open(&mut world, id);
    }
    // Build the stack [2, 3, 1] with 1 focused.
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    world.handle_action(Action::FocusColumn(Horizontal::Left));
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Right));

    world.handle_event(Event::WindowClosed { id: WindowId(3) });
    assert_eq!(focused(&world), Some(1));
    assert_eq!(placement(&world, 2).rect, Rect::new(10, 10, 485, 285));
    assert_eq!(placement(&world, 1).rect, Rect::new(10, 305, 485, 285));
}

#[test]
fn close_is_a_request_until_the_platform_confirms() {
    let mut world = world();
    open(&mut world, 1);
    let effects = world.handle_action(Action::CloseFocused);
    assert_eq!(effects, vec![Effect::Close(WindowId(1))]);
    assert!(world.arrange().get(WindowId(1)).is_some());
}

#[test]
fn consume_and_expel_round_trip() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);

    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    assert_eq!(placement(&world, 1).rect, Rect::new(10, 10, 485, 285));
    assert_eq!(placement(&world, 2).rect, Rect::new(10, 305, 485, 285));
    assert_eq!(focused(&world), Some(2));

    world.handle_action(Action::ConsumeOrExpel(Horizontal::Right));
    assert_eq!(placement(&world, 1).rect, Rect::new(10, 10, 485, 580));
    assert_eq!(placement(&world, 2).rect, Rect::new(505, 10, 485, 580));
}

#[test]
fn minimum_heights_are_respected() {
    let mut world = world();
    open_with(&mut world, 1, min_height(400));
    open(&mut world, 2);
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    assert_eq!(placement(&world, 1).rect.h, 400);
    assert_eq!(placement(&world, 2).rect, Rect::new(10, 420, 485, 170));
}

#[test]
fn stacked_windows_never_get_zero_height() {
    let mut world = world();
    open_with(&mut world, 1, min_height(600));
    open(&mut world, 2);
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    assert_eq!(placement(&world, 2).rect.h, 1);
}

#[test]
fn move_column_carries_focus() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::MoveColumn(Horizontal::Left));
    assert_eq!(focused(&world), Some(2));
    assert_eq!(placement(&world, 2).rect.x, 10);
    assert_eq!(placement(&world, 1).rect.x, 505);
}

#[test]
fn focus_and_move_within_a_column() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));

    world.handle_action(Action::FocusWindow(Vertical::Up));
    assert_eq!(focused(&world), Some(1));

    world.handle_action(Action::MoveWindow(Vertical::Down));
    assert_eq!(focused(&world), Some(1));
    assert_eq!(placement(&world, 1).rect.y, 305);
    assert_eq!(placement(&world, 2).rect.y, 10);
}

#[test]
fn column_width_cycles_through_presets() {
    let mut world = world();
    open(&mut world, 1);
    world.handle_action(Action::CycleColumnWidth);
    assert_eq!(placement(&world, 1).rect.w, 980);
    world.handle_action(Action::CycleColumnWidth);
    assert_eq!(placement(&world, 1).rect.w, 485);
}

#[test]
fn set_column_width_lands_on_the_named_preset() {
    // The absolute half of the cycle above: one action lands on entry N of
    // the width list instead of stepping past every other entry. The test
    // world's list is `[0.5, 1.0]`, so index 1 is full width.
    let mut world = world();
    open(&mut world, 1);
    assert_eq!(placement(&world, 1).rect.w, 485);

    assert!(world.handle_action(Action::SetColumnWidth(1)).is_empty());
    assert_eq!(placement(&world, 1).rect.w, 980);
    assert_eq!(focused(&world), Some(1));

    world.handle_action(Action::SetColumnWidth(0));
    assert_eq!(placement(&world, 1).rect.w, 485);
    assert_eq!(focused(&world), Some(1));
}

#[test]
fn an_out_of_range_column_width_index_is_ignored() {
    let mut world = world();
    open(&mut world, 1);
    let before = placement(&world, 1);
    // One past the end, and the two extremes a client could send over a
    // protocol that takes an unbounded number -- mirroring the
    // workspace-index case, including the promise nothing moves.
    for index in [2, usize::MAX / 2, usize::MAX] {
        assert!(
            world
                .handle_action(Action::SetColumnWidth(index))
                .is_empty()
        );
        assert_eq!(placement(&world, 1), before, "index {index}");
        assert_eq!(focused(&world), Some(1), "index {index}");
    }
}

#[test]
fn an_out_of_range_set_keeps_learned_widths() {
    // The `handle_action` guard's pin: an ignored index must change nothing
    // at all, not even the learned minimums a *landed* set clears (see the
    // next test). Without the guard this narrows to the preset width.
    let mut world = world();
    open(&mut world, 1);
    world.handle_event(Event::FrameObserved {
        id: WindowId(1),
        requested: Size::new(485, 580),
        actual: Size::new(600, 580),
    });
    assert_eq!(placement(&world, 1).rect.w, 600);
    world.handle_action(Action::SetColumnWidth(usize::MAX));
    assert_eq!(placement(&world, 1).rect.w, 600);
}

#[test]
fn set_column_width_forgets_learned_widths() {
    // Choosing a width on purpose overrides what frames taught, exactly like
    // `choosing_a_width_forgets_learned_widths` pins for the cycle.
    let mut world = world();
    open(&mut world, 1);
    world.handle_event(Event::FrameObserved {
        id: WindowId(1),
        requested: Size::new(485, 580),
        actual: Size::new(600, 580),
    });
    world.handle_action(Action::SetColumnWidth(0));
    assert_eq!(placement(&world, 1).rect.w, 485);
}

#[test]
fn set_column_width_on_an_empty_workspace_does_nothing() {
    // No column to resize: the action must not reach one that isn't there.
    let mut world = world();
    world.handle_action(Action::SetColumnWidth(0));
    world.handle_action(Action::SetColumnWidth(3));
    assert_eq!(focused(&world), None);
    assert!(world.arrange().placements.is_empty());
}

#[test]
fn set_column_width_with_no_outputs_does_nothing() {
    // Nothing to index into: the action must not reach a tree that isn't
    // there. `reshape` already guards this, but it is the shape a protocol
    // client can provoke (bind, then the output goes away).
    let mut world = World::new(config());
    world.handle_action(Action::SetColumnWidth(0));
    world.handle_action(Action::SetColumnWidth(usize::MAX));
    assert_eq!(world.workspaces(OutputId(1)), None);
}

#[test]
fn layout_actions_on_an_empty_workspace_do_nothing() {
    let mut world = world();
    let actions = [
        Action::FocusColumn(Horizontal::Left),
        Action::ScrollViewToColumn(0),
        Action::ScrollViewToColumn(3),
        Action::MoveColumn(Horizontal::Right),
        Action::FocusWindow(Vertical::Up),
        Action::MoveWindow(Vertical::Down),
        Action::ConsumeOrExpel(Horizontal::Left),
        Action::CycleColumnWidth,
        Action::SetColumnWidth(0),
        Action::SetColumnWidth(3),
        Action::FocusWorkspace(Vertical::Down),
        Action::MoveWindowToWorkspace(Vertical::Down),
        Action::CloseFocused,
    ];
    for action in actions {
        assert!(world.handle_action(action).is_empty());
    }
    assert_eq!(focused(&world), None);
    assert!(world.arrange().placements.is_empty());
}

#[test]
fn actions_without_an_output_do_nothing() {
    let mut world = World::new(config());
    assert!(
        world
            .handle_action(Action::MoveColumn(Horizontal::Left))
            .is_empty()
    );
    assert!(
        world
            .handle_action(Action::FocusWorkspace(Vertical::Down))
            .is_empty()
    );
    assert_eq!(focused(&world), None);
}

#[test]
fn arrange_is_pure() {
    let mut world = world();
    for id in 1..=3 {
        open(&mut world, id);
    }
    assert_eq!(world.arrange(), world.arrange());
}

#[test]
fn scroll_view_to_column_lines_the_left_edge_up_with_two_visible() {
    // The ticket's shape: three half-width columns, 1000x600 with a 10px
    // gap, so two are visible. Focusing the neighbour moves nothing
    // (`scroll_into_view` is already satisfied); aligning scrolls it to
    // the usable edge.
    let mut world = world();
    for id in 1..=3 {
        open(&mut world, id);
    }
    // Focus 1, so column 2 is the already-visible neighbour.
    world.handle_action(Action::FocusColumn(Horizontal::Left));
    world.handle_action(Action::FocusColumn(Horizontal::Left));
    assert_eq!(focused(&world), Some(1));
    assert_eq!(placement(&world, 1).rect.x, 10);
    assert_eq!(placement(&world, 2).rect.x, 505);
    // Plain focus would not move the view here: check the premise.
    world.handle_action(Action::FocusColumn(Horizontal::Right));
    assert_eq!(focused(&world), Some(2));
    assert_eq!(placement(&world, 2).rect.x, 505);
    // Back to 1, then align to column 2 (index 1).
    world.handle_action(Action::FocusColumn(Horizontal::Left));
    world.handle_action(Action::ScrollViewToColumn(1));
    assert_eq!(focused(&world), Some(2));
    assert_eq!(placement(&world, 2).rect.x, 10);
    assert_eq!(placement(&world, 3).rect.x, 505);
}

#[test]
fn scroll_view_to_column_clamps_at_both_ends() {
    let mut world = world();
    for id in 1..=4 {
        open(&mut world, id);
    }
    // First column aligns to the usable edge (already there).
    world.handle_action(Action::ScrollViewToColumn(0));
    assert_eq!(focused(&world), Some(1));
    assert_eq!(placement(&world, 1).rect.x, 10);
    // Last column cannot align its left edge: the strip ends, so it clamps
    // to the strip's end like every other scroll.
    world.handle_action(Action::ScrollViewToColumn(3));
    assert_eq!(focused(&world), Some(4));
    // Strip: 4 x 485 + 3 x 10 = 1970; view 980 wide; max scroll 990.
    // Column 4 starts at 3*(485+10) = 1485 in strip coords, so view 990
    // puts it at x 505, and column 1 at -980.
    assert_eq!(placement(&world, 4).rect.x, 505);
    assert_eq!(placement(&world, 1).rect.x, 10 - 990);
}

#[test]
fn scroll_view_to_column_with_one_column_is_a_noop() {
    let mut world = world();
    open(&mut world, 1);
    let before = placement(&world, 1);
    world.handle_action(Action::ScrollViewToColumn(0));
    assert_eq!(focused(&world), Some(1));
    assert_eq!(placement(&world, 1), before);
    assert_eq!(before.rect.x, 10);
}

#[test]
fn scroll_view_to_column_out_of_range_changes_nothing() {
    let mut world = world();
    for id in 1..=2 {
        open(&mut world, id);
    }
    let before = world.arrange();
    for index in [2, 3, usize::MAX / 2, usize::MAX] {
        assert!(
            world
                .handle_action(Action::ScrollViewToColumn(index))
                .is_empty()
        );
        assert_eq!(world.arrange(), before, "index {index}");
        assert_eq!(focused(&world), Some(2), "index {index}");
    }
}

#[test]
fn scroll_view_to_column_with_no_columns_or_outputs_does_nothing() {
    let mut world = world();
    world.handle_action(Action::ScrollViewToColumn(0));
    world.handle_action(Action::ScrollViewToColumn(3));
    assert_eq!(focused(&world), None);
    assert!(world.arrange().placements.is_empty());
    let mut bare = World::new(config());
    bare.handle_action(Action::ScrollViewToColumn(0));
    bare.handle_action(Action::ScrollViewToColumn(usize::MAX));
    assert_eq!(bare.workspaces(OutputId(1)), None);
}

#[test]
fn scroll_view_to_column_on_an_already_aligned_column_changes_nothing_visible() {
    let mut world = world();
    for id in 1..=3 {
        open(&mut world, id);
    }
    world.handle_action(Action::ScrollViewToColumn(2));
    let before = world.arrange();
    world.handle_action(Action::ScrollViewToColumn(2));
    assert_eq!(world.arrange(), before);
    assert_eq!(focused(&world), Some(3));
}

#[test]
fn scroll_view_to_column_with_different_widths_aligns_each_start() {
    let mut world = world();
    open(&mut world, 1);
    world.handle_action(Action::SetColumnWidth(1));
    open(&mut world, 2);
    open(&mut world, 3);
    // Column 1 is now full width (980); columns 2-3 half (485).
    world.handle_action(Action::ScrollViewToColumn(0));
    assert_eq!(placement(&world, 1).rect.x, 10);
    assert_eq!(placement(&world, 1).rect.w, 980);
    world.handle_action(Action::ScrollViewToColumn(1));
    assert_eq!(focused(&world), Some(2));
    assert_eq!(placement(&world, 2).rect.x, 10);
}

#[test]
fn scroll_view_to_column_with_a_short_strip_stays_put() {
    let mut world = world();
    open(&mut world, 1);
    let before = world.arrange();
    world.handle_action(Action::ScrollViewToColumn(0));
    assert_eq!(world.arrange(), before);
}

#[test]
fn scroll_view_to_column_on_fullscreen_and_maximized_columns() {
    let mut world = world();
    for id in 1..=3 {
        open(&mut world, id);
    }
    world.handle_action(Action::ToggleFullscreen);
    assert_eq!(world.fullscreen_on(OutputId(1)), Some(WindowId(3)));
    world.handle_action(Action::ScrollViewToColumn(2));
    assert_eq!(focused(&world), Some(3));
    assert_eq!(world.fullscreen_on(OutputId(1)), Some(WindowId(3)));
    world.handle_action(Action::ToggleFullscreen);
    assert_eq!(world.fullscreen_on(OutputId(1)), None);
    world.handle_action(Action::ToggleMaximize);
    assert_eq!(world.maximized_on(OutputId(1)), Some(WindowId(3)));
    world.handle_action(Action::ScrollViewToColumn(0));
    assert_eq!(focused(&world), Some(1));
    assert_eq!(world.maximized_on(OutputId(1)), None);
    world.handle_action(Action::ScrollViewToColumn(2));
    assert_eq!(focused(&world), Some(3));
    assert_eq!(world.maximized_on(OutputId(1)), Some(WindowId(3)));
}

#[test]
fn scroll_view_to_column_leaves_the_floating_layer_for_the_strip() {
    let mut world = world();
    for id in 1..=2 {
        open(&mut world, id);
    }
    world.handle_action(Action::ToggleFloating);
    assert!(world.floating_has_focus());
    world.handle_action(Action::ScrollViewToColumn(0));
    assert!(!world.floating_has_focus());
    assert_eq!(focused(&world), Some(1));
    assert_eq!(placement(&world, 1).rect.x, 10);
}
