//! Floating: where a floating window goes, what it does to the strip (nothing
//! but take or give back its column), focus between the two layers, and how
//! it travels and interacts with fullscreen.
//!
//! The screen is the shared 1000x600 with a 10px gap: a 980x580 usable area
//! at (10, 10), half-width columns 485px wide.

use super::*;
use crate::{Action, Horizontal, Size, Vertical};

const BAR: Rect = Rect::new(0, 30, 1000, 570);

fn float_on_map(world: &mut World, id: u64) {
    world.handle_event(Event::FloatingRequested {
        id: WindowId(id),
        floating: true,
        size: None,
    });
}

fn draw(world: &mut World, id: u64, w: i32, h: i32) {
    world.handle_event(Event::FrameObserved {
        id: WindowId(id),
        requested: Size::default(),
        actual: Size::new(w, h),
    });
}

fn with_parent(parent: u64) -> WindowInfo {
    WindowInfo {
        parent: Some(WindowId(parent)),
        ..WindowInfo::default()
    }
}

/// Every tiled placement, as (id, rect, visible): what "the strip" looks like.
fn strip(world: &World) -> Vec<(WindowId, Rect, bool)> {
    world
        .arrange()
        .placements
        .iter()
        .filter(|p| !p.floating)
        .map(|p| (p.id, p.rect, p.visible))
        .collect()
}

/// The ids of the focused workspace's floating layer, bottom first.
fn floating_stack(world: &World) -> Vec<u64> {
    world.outputs[world.focused_output]
        .active_workspace()
        .floating
        .iter()
        .map(|id| id.0)
        .collect()
}

/// A world with columns 1..=n open and the strip focus on `focus`.
fn strip_of(n: u64, focus: u64) -> World {
    let mut world = world();
    for id in 1..=n {
        open(&mut world, id);
        draw(&mut world, id, 485, 580);
    }
    world.handle_action(Action::FocusWindowId(WindowId(focus)));
    world
}

#[test]
fn a_window_floating_as_it_maps_leaves_the_strip_exactly_as_it_was() {
    // Five columns, the second focused and the view scrolled to it: the new
    // window goes in right of it and scrolls the strip, and floating it must
    // put both focus and scroll back.
    for focus in 1..=5 {
        let mut world = strip_of(5, focus);
        let before = strip(&world);
        let focused_before = focused(&world);
        open(&mut world, 9);
        assert_ne!(strip(&world), before, "the column went in first");
        float_on_map(&mut world, 9);
        assert_eq!(strip(&world), before, "strip changed (focus was {focus})");
        assert_eq!(focused(&world), Some(9), "the dialog has focus");
        // And the strip's own focus is where it was: toggling back to it
        // lands on the same window.
        world.handle_action(Action::ToggleFloatingFocus);
        assert_eq!(focused(&world), focused_before);
        assert_eq!(strip(&world), before);
    }
}

#[test]
fn a_floating_window_is_invisible_until_it_draws_then_centred() {
    let mut world = strip_of(2, 1);
    open(&mut world, 9);
    float_on_map(&mut world, 9);
    let placed = placement(&world, 9);
    assert!(placed.floating);
    assert!(
        !placed.visible,
        "nothing to show before a frame: {placed:?}"
    );
    assert_eq!(placed.requested, None, "the window chooses its own size");
    draw(&mut world, 9, 300, 200);
    let placed = placement(&world, 9);
    assert!(placed.visible);
    // Centred on the usable area (the whole screen here): (500, 300).
    assert_eq!(placed.rect, Rect::new(350, 200, 300, 200));
    assert_eq!(placed.requested, None);
    // It keeps its centre as it resizes itself.
    draw(&mut world, 9, 400, 100);
    assert_eq!(placement(&world, 9).rect, Rect::new(300, 250, 400, 100));
}

#[test]
fn an_initial_size_places_it_before_it_draws_and_is_asked_for() {
    let mut world = strip_of(1, 1);
    open(&mut world, 9);
    world.handle_event(Event::FloatingRequested {
        id: WindowId(9),
        floating: true,
        size: Some(Size::new(600, 400)),
    });
    let placed = placement(&world, 9);
    assert!(placed.visible);
    assert_eq!(placed.rect, Rect::new(200, 100, 600, 400));
    assert_eq!(placed.requested, Some(Size::new(600, 400)));
    // A size past the usable area is clamped to it, both in the rect and in
    // what is asked for; a zero or negative size asks for nothing.
    let mut world = strip_of(1, 1);
    open(&mut world, 9);
    world.handle_event(Event::FloatingRequested {
        id: WindowId(9),
        floating: true,
        size: Some(Size::new(5000, i32::MAX)),
    });
    assert_eq!(placement(&world, 9).requested, Some(Size::new(1000, 600)));
    assert_eq!(placement(&world, 9).rect, SCREEN);
    let mut world = strip_of(1, 1);
    open(&mut world, 9);
    world.handle_event(Event::FloatingRequested {
        id: WindowId(9),
        floating: true,
        size: Some(Size::new(0, -5)),
    });
    assert_eq!(placement(&world, 9).requested, None);
}

#[test]
fn a_dialog_is_centred_on_its_parent() {
    let mut world = strip_of(2, 1);
    let parent = placement(&world, 1).rect;
    open_with(&mut world, 9, with_parent(1));
    float_on_map(&mut world, 9);
    draw(&mut world, 9, 200, 100);
    let placed = placement(&world, 9).rect;
    assert_eq!(
        (placed.x + placed.w / 2, placed.y + placed.h / 2),
        (parent.x + parent.w / 2, parent.y + parent.h / 2),
        "{placed:?} not centred on {parent:?}"
    );
    // A parent that is not visible (scrolled away) or unknown centres it on
    // the output instead.
    let mut world = strip_of(6, 1);
    world.handle_action(Action::FocusWindowId(WindowId(6)));
    assert!(!placement(&world, 1).visible);
    open_with(&mut world, 9, with_parent(1));
    float_on_map(&mut world, 9);
    draw(&mut world, 9, 200, 100);
    assert_eq!(placement(&world, 9).rect, Rect::new(400, 250, 200, 100));
    let mut world = strip_of(1, 1);
    open_with(&mut world, 9, with_parent(12345));
    float_on_map(&mut world, 9);
    draw(&mut world, 9, 200, 100);
    assert_eq!(placement(&world, 9).rect, Rect::new(400, 250, 200, 100));
}

#[test]
fn a_dialog_of_a_dialog_centres_on_the_first_one() {
    // The first dialog is centred on column 1, the left half of the screen,
    // so it is off the output's centre -- the second must follow it there
    // rather than the usable area.
    let mut world = strip_of(2, 1);
    open_with(&mut world, 8, with_parent(1));
    float_on_map(&mut world, 8);
    draw(&mut world, 8, 400, 300);
    let first = placement(&world, 8).rect;
    assert_ne!(first.x + first.w / 2, 500, "not off centre: {first:?}");
    open_with(&mut world, 9, with_parent(8));
    float_on_map(&mut world, 9);
    draw(&mut world, 9, 100, 50);
    let second = placement(&world, 9).rect;
    assert_eq!(
        (second.x + second.w / 2, second.y + second.h / 2),
        (first.x + first.w / 2, first.y + first.h / 2)
    );
    assert_eq!(floating_stack(&world), vec![8, 9]);
}

#[test]
fn floating_windows_stay_inside_the_usable_area() {
    let mut world = strip_of(1, 1);
    world.handle_event(Event::OutputUsableAreaChanged {
        id: OutputId(1),
        area: BAR,
    });
    open(&mut world, 9);
    float_on_map(&mut world, 9);
    // Taller than the usable area: placed at its full height, and asked to
    // fit from then on.
    draw(&mut world, 9, 300, 900);
    let placed = placement(&world, 9);
    assert_eq!(placed.rect, Rect::new(350, 30, 300, 570));
    assert_eq!(placed.requested, Some(Size::new(300, 570)));
    // The window complies, and the ask sticks rather than going back to
    // "choose your own" (which would let it grow again, every frame).
    draw(&mut world, 9, 300, 570);
    assert_eq!(placement(&world, 9).requested, Some(Size::new(300, 570)));
}

#[test]
fn an_off_centre_parent_near_the_edge_keeps_the_dialog_on_screen() {
    // The parent is the rightmost column, so centring a wide dialog on it
    // would run off the right edge: it is shifted in, not cut.
    let mut world = strip_of(2, 2);
    open_with(&mut world, 9, with_parent(2));
    float_on_map(&mut world, 9);
    draw(&mut world, 9, 900, 100);
    let placed = placement(&world, 9).rect;
    assert!(placed.x >= 0 && placed.right() <= 1000, "{placed:?}");
    assert_eq!(placed.w, 900);
}

#[test]
fn focusing_a_floating_window_raises_it() {
    let mut world = strip_of(1, 1);
    for id in [7, 8, 9] {
        open(&mut world, id);
        float_on_map(&mut world, id);
        draw(&mut world, id, 100 * id as i32, 100);
    }
    assert_eq!(floating_stack(&world), vec![7, 8, 9]);
    world.handle_action(Action::FocusWindowId(WindowId(7)));
    assert_eq!(floating_stack(&world), vec![8, 9, 7]);
    assert_eq!(focused(&world), Some(7));
    // A click the platform reports raises the same way.
    world.handle_event(Event::FocusObserved { id: WindowId(8) });
    assert_eq!(floating_stack(&world), vec![9, 7, 8]);
    // The arrangement stacks them in that order, above the strip.
    let order: Vec<u64> = world.arrange().placements.iter().map(|p| p.id.0).collect();
    assert_eq!(order, vec![1, 9, 7, 8]);
    // Focusing the strip's window leaves the stack alone.
    world.handle_action(Action::FocusWindowId(WindowId(1)));
    assert_eq!(focused(&world), Some(1));
    assert_eq!(floating_stack(&world), vec![9, 7, 8]);
}

#[test]
fn focus_moves_between_the_floating_layer_and_the_strip() {
    let mut world = strip_of(3, 2);
    open(&mut world, 9);
    float_on_map(&mut world, 9);
    draw(&mut world, 9, 100, 100);
    assert_eq!(focused(&world), Some(9));
    world.handle_action(Action::ToggleFloatingFocus);
    assert_eq!(focused(&world), Some(2));
    world.handle_action(Action::ToggleFloatingFocus);
    assert_eq!(focused(&world), Some(9));
    // focus-column leaves the floating layer for the strip's focused
    // column, without stepping, whichever way it points.
    for dir in [Horizontal::Left, Horizontal::Right] {
        world.handle_action(Action::FocusWindowId(WindowId(9)));
        world.handle_action(Action::FocusColumn(dir));
        assert_eq!(focused(&world), Some(2));
    }
    // With no floating window, the toggle does nothing.
    let mut world = strip_of(2, 1);
    let before = world.arrange();
    world.handle_action(Action::ToggleFloatingFocus);
    assert_eq!(world.arrange(), before);
}

#[test]
fn a_workspace_with_only_floating_windows_focuses_them() {
    let mut world = world();
    open(&mut world, 9);
    float_on_map(&mut world, 9);
    assert_eq!(focused(&world), Some(9));
    // The toggle cannot leave: there is no strip to go to.
    world.handle_action(Action::ToggleFloatingFocus);
    assert_eq!(focused(&world), Some(9));
    world.handle_action(Action::FocusColumn(Horizontal::Left));
    assert_eq!(focused(&world), Some(9));
    // It is not an empty workspace: switching away and back keeps it.
    world.handle_action(Action::FocusWorkspace(Vertical::Down));
    assert_eq!(focused(&world), None);
    world.handle_action(Action::FocusWorkspace(Vertical::Up));
    assert_eq!(focused(&world), Some(9));
    assert_eq!(workspace_count(&world), 2);
}

#[test]
fn focus_window_cycles_the_floating_stack() {
    let mut world = strip_of(1, 1);
    for id in [7, 8, 9] {
        open(&mut world, id);
        float_on_map(&mut world, id);
    }
    world.handle_action(Action::FocusWindow(Vertical::Down));
    assert_eq!(floating_stack(&world), vec![8, 9, 7]);
    assert_eq!(focused(&world), Some(7));
    world.handle_action(Action::FocusWindow(Vertical::Down));
    world.handle_action(Action::FocusWindow(Vertical::Down));
    assert_eq!(floating_stack(&world), vec![7, 8, 9]);
    world.handle_action(Action::FocusWindow(Vertical::Up));
    assert_eq!(floating_stack(&world), vec![9, 7, 8]);
    assert_eq!(focused(&world), Some(8));
}

#[test]
fn strip_actions_do_nothing_while_a_floating_window_has_focus() {
    let mut world = strip_of(3, 2);
    open(&mut world, 9);
    float_on_map(&mut world, 9);
    draw(&mut world, 9, 100, 100);
    let before = world.arrange();
    for action in [
        Action::MoveColumn(Horizontal::Left),
        Action::MoveColumn(Horizontal::Right),
        Action::MoveWindow(Vertical::Up),
        Action::ConsumeOrExpel(Horizontal::Left),
        Action::ConsumeOrExpel(Horizontal::Right),
        Action::CycleColumnWidth,
        Action::SetColumnWidth(1),
    ] {
        world.handle_action(action.clone());
        assert_eq!(world.arrange(), before, "{action:?} changed the layout");
    }
}

#[test]
fn toggling_floats_the_focused_column_and_puts_it_back_where_it_was() {
    let mut world = strip_of(4, 2);
    world.handle_action(Action::SetColumnWidth(1));
    let before = strip(&world);
    world.handle_action(Action::ToggleFloating);
    assert!(world.is_floating(WindowId(2)));
    assert_eq!(focused(&world), Some(2), "floating keeps focus");
    // It keeps the size it drew until it draws another: the full-width
    // column it was, clamped to the screen.
    let placed = placement(&world, 2);
    assert!(placed.visible && placed.floating);
    assert_eq!(placed.requested, None);
    // The strip's focus went left, to 1.
    world.handle_action(Action::ToggleFloatingFocus);
    assert_eq!(focused(&world), Some(1));
    world.handle_action(Action::FocusWindowId(WindowId(2)));
    world.handle_action(Action::ToggleFloating);
    assert!(!world.is_floating(WindowId(2)));
    assert_eq!(focused(&world), Some(2));
    // Same column order and width; the scroll may differ, the rects'
    // widths and order must not.
    let after = strip(&world);
    let shape = |s: &[(WindowId, Rect, bool)]| -> Vec<(WindowId, i32)> {
        s.iter().map(|(id, rect, _)| (*id, rect.w)).collect()
    };
    assert_eq!(shape(&after), shape(&before));
}

#[test]
fn un_floating_goes_right_of_the_strip_focus() {
    let mut world = strip_of(3, 1);
    open(&mut world, 9);
    float_on_map(&mut world, 9);
    world.handle_action(Action::ToggleFloatingFocus);
    world.handle_action(Action::FocusWindowId(WindowId(2)));
    // By id, while the strip has focus: it goes in right of 2, unfocused.
    world.handle_action(Action::SetFloating {
        id: WindowId(9),
        floating: false,
    });
    let order: Vec<u64> = strip(&world).iter().map(|(id, _, _)| id.0).collect();
    assert_eq!(order, vec![1, 2, 9, 3]);
    assert_eq!(focused(&world), Some(2));
    // A window that floated as it opened never had a width: the default.
    assert_eq!(placement(&world, 9).rect.w, placement(&world, 1).rect.w);
}

#[test]
fn floating_by_id_does_not_steal_focus() {
    let mut world = strip_of(3, 1);
    world.handle_action(Action::SetFloating {
        id: WindowId(3),
        floating: true,
    });
    assert_eq!(focused(&world), Some(1));
    assert_eq!(floating_stack(&world), vec![3]);
    // With a floating window focused, the next one goes under it.
    world.handle_action(Action::FocusWindowId(WindowId(3)));
    world.handle_action(Action::SetFloating {
        id: WindowId(2),
        floating: true,
    });
    assert_eq!(focused(&world), Some(3));
    assert_eq!(floating_stack(&world), vec![2, 3]);
}

#[test]
fn the_asked_for_state_or_an_unknown_id_changes_nothing() {
    let mut world = strip_of(2, 1);
    open(&mut world, 9);
    float_on_map(&mut world, 9);
    let before = world.arrange();
    float_on_map(&mut world, 9);
    world.handle_action(Action::SetFloating {
        id: WindowId(9),
        floating: true,
    });
    world.handle_action(Action::SetFloating {
        id: WindowId(1),
        floating: false,
    });
    world.handle_action(Action::SetFloating {
        id: WindowId(u64::MAX),
        floating: true,
    });
    world.handle_event(Event::FloatingRequested {
        id: WindowId(404),
        floating: true,
        size: Some(Size::new(10, 10)),
    });
    assert_eq!(world.arrange(), before);
    assert!(!world.is_floating(WindowId(404)));
}

#[test]
fn floating_frames_never_teach_the_strip_a_minimum() {
    let mut world = strip_of(2, 1);
    world.handle_action(Action::ToggleFloating);
    world.handle_event(Event::FrameObserved {
        id: WindowId(1),
        requested: Size::new(100, 100),
        actual: Size::new(900, 500),
    });
    world.handle_action(Action::ToggleFloating);
    assert_eq!(
        placement(&world, 1).rect.w,
        placement(&world, 2).rect.w,
        "the column came back wider"
    );
}

#[test]
fn moving_a_floating_window_to_another_workspace_keeps_it_floating() {
    let mut world = strip_of(2, 1);
    open(&mut world, 9);
    float_on_map(&mut world, 9);
    draw(&mut world, 9, 300, 200);
    let rect = placement(&world, 9).rect;
    let before = strip(&world);
    world.handle_action(Action::MoveWindowToWorkspace(Vertical::Down));
    assert!(world.is_floating(WindowId(9)));
    assert_eq!(focused(&world), Some(9));
    assert_eq!(placement(&world, 9).rect, rect, "same output, same place");
    // The strip it left is untouched (just no longer on screen).
    let left: Vec<_> = before.iter().map(|(id, r, _)| (*id, *r)).collect();
    let now: Vec<_> = strip(&world).iter().map(|(id, r, _)| (*id, *r)).collect();
    assert_eq!(now, left);
    world.handle_action(Action::MoveWindowToWorkspaceIndex(0));
    assert_eq!(floating_stack(&world), vec![9]);
}

#[test]
fn moving_a_floating_window_to_another_output_re_centres_it_there() {
    let mut world = strip_of(1, 1);
    world.handle_event(Event::OutputAdded {
        id: OutputId(2),
        area: Rect::new(1000, 0, 1600, 900),
    });
    open_with(&mut world, 9, with_parent(1));
    float_on_map(&mut world, 9);
    draw(&mut world, 9, 200, 100);
    world.handle_action(Action::MoveFocusedWindowToOutput(OutputId(2)));
    let placed = placement(&world, 9);
    assert_eq!(placed.output, OutputId(2));
    assert!(placed.floating && placed.visible);
    assert_eq!(placed.rect, Rect::new(1700, 400, 200, 100));
    assert_eq!(focused(&world), Some(9));
}

#[test]
fn an_unplugged_output_s_floating_windows_survive() {
    let mut world = world();
    world.handle_event(Event::OutputAdded {
        id: OutputId(2),
        area: Rect::new(1000, 0, 800, 600),
    });
    world.handle_action(Action::FocusOutput(OutputId(2)));
    open(&mut world, 9);
    float_on_map(&mut world, 9);
    draw(&mut world, 9, 100, 100);
    world.handle_event(Event::OutputRemoved { id: OutputId(2) });
    assert!(world.is_floating(WindowId(9)));
    assert_eq!(placement(&world, 9).output, OutputId(1));
    // With no output left at all, it waits -- still floating -- and comes
    // back floating.
    world.handle_event(Event::OutputRemoved { id: OutputId(1) });
    assert!(world.arrange().placements.is_empty());
    world.handle_event(Event::OutputAdded {
        id: OutputId(3),
        area: SCREEN,
    });
    let placed = placement(&world, 9);
    assert!(placed.floating && placed.visible, "{placed:?}");
}

#[test]
fn a_window_floated_before_any_output_exists_arrives_floating() {
    let mut world = World::new(config());
    open(&mut world, 9);
    float_on_map(&mut world, 9);
    assert!(world.is_floating(WindowId(9)));
    world.handle_event(Event::OutputAdded {
        id: OutputId(1),
        area: SCREEN,
    });
    assert!(placement(&world, 9).floating);
}

#[test]
fn a_floating_window_goes_fullscreen_and_comes_back() {
    let mut world = strip_of(2, 1);
    open(&mut world, 9);
    float_on_map(&mut world, 9);
    draw(&mut world, 9, 300, 200);
    let floated = placement(&world, 9);
    let strip_before = strip(&world);
    world.handle_action(Action::ToggleFullscreen);
    assert_eq!(world.fullscreen_on(OutputId(1)), Some(WindowId(9)));
    let placed = placement(&world, 9);
    assert!(placed.fullscreen && placed.visible && placed.floating);
    assert_eq!(placed.rect, SCREEN);
    assert!(
        strip(&world).iter().all(|(_, _, visible)| !visible),
        "the strip shows under a covering floating window"
    );
    // Focus the strip: the fullscreen floating window hides, the strip shows.
    world.handle_action(Action::ToggleFloatingFocus);
    assert_eq!(world.fullscreen_on(OutputId(1)), None);
    assert!(!placement(&world, 9).visible);
    assert_eq!(strip(&world), strip_before);
    world.handle_action(Action::ToggleFloatingFocus);
    // It draws at the output's size while fullscreen, and leaves back where
    // it floated once it draws its own size again.
    draw(&mut world, 9, 1000, 600);
    world.handle_action(Action::ToggleFullscreen);
    assert!(world.is_floating(WindowId(9)));
    assert_eq!(placement(&world, 9).requested, None, "no clamp was learned");
    draw(&mut world, 9, 300, 200);
    assert_eq!(placement(&world, 9), floated);
    assert_eq!(strip(&world), strip_before);
}

#[test]
fn a_dialog_over_a_covering_fullscreen_window_shows_while_it_has_focus() {
    let mut world = strip_of(2, 1);
    world.handle_action(Action::ToggleFullscreen);
    assert_eq!(world.fullscreen_on(OutputId(1)), Some(WindowId(1)));
    open_with(&mut world, 9, with_parent(1));
    float_on_map(&mut world, 9);
    draw(&mut world, 9, 300, 200);
    // The dialog has focus and shows; nothing covers the output while it
    // does, and the fullscreen window stays where it was, full size.
    assert_eq!(focused(&world), Some(9));
    assert!(placement(&world, 9).visible);
    assert_eq!(world.fullscreen_on(OutputId(1)), None);
    let video = placement(&world, 1);
    assert!(video.visible && video.fullscreen);
    assert_eq!(video.rect, SCREEN);
    // Back to the fullscreen window: it covers again, and the dialog hides.
    world.handle_action(Action::ToggleFloatingFocus);
    assert_eq!(world.fullscreen_on(OutputId(1)), Some(WindowId(1)));
    assert!(!placement(&world, 9).visible);
    // Closing the dialog while it has focus returns focus to its parent,
    // which covers again.
    world.handle_action(Action::ToggleFloatingFocus);
    world.handle_event(Event::WindowClosed { id: WindowId(9) });
    assert_eq!(focused(&world), Some(1));
    assert_eq!(world.fullscreen_on(OutputId(1)), Some(WindowId(1)));
}

#[test]
fn floating_or_un_floating_a_fullscreen_window_ends_its_fullscreen() {
    let mut world = strip_of(2, 1);
    world.handle_action(Action::ToggleFullscreen);
    world.handle_action(Action::ToggleFloating);
    assert!(world.is_floating(WindowId(1)));
    assert!(!world.is_fullscreen(WindowId(1)));
    world.handle_action(Action::ToggleFullscreen);
    world.handle_action(Action::ToggleFloating);
    assert!(!world.is_floating(WindowId(1)));
    assert!(!world.is_fullscreen(WindowId(1)));
}

#[test]
fn closing_a_focused_floating_window_hands_focus_back() {
    // To a tiled parent.
    let mut world = strip_of(3, 3);
    open_with(&mut world, 9, with_parent(1));
    float_on_map(&mut world, 9);
    world.handle_event(Event::WindowClosed { id: WindowId(9) });
    assert_eq!(focused(&world), Some(1));
    // To a floating parent, raising it over another floating window.
    let mut world = strip_of(1, 1);
    open(&mut world, 7);
    float_on_map(&mut world, 7);
    open(&mut world, 8);
    float_on_map(&mut world, 8);
    open_with(&mut world, 9, with_parent(7));
    float_on_map(&mut world, 9);
    world.handle_event(Event::WindowClosed { id: WindowId(9) });
    assert_eq!(focused(&world), Some(7));
    assert_eq!(floating_stack(&world), vec![8, 7]);
    // No parent: the next floating window, then the strip.
    world.handle_event(Event::WindowClosed { id: WindowId(7) });
    assert_eq!(focused(&world), Some(8));
    world.handle_event(Event::WindowClosed { id: WindowId(8) });
    assert_eq!(focused(&world), Some(1));
    // A floating window that was not focused closing moves no focus.
    let mut world = strip_of(2, 2);
    open(&mut world, 9);
    float_on_map(&mut world, 9);
    world.handle_action(Action::ToggleFloatingFocus);
    world.handle_event(Event::WindowClosed { id: WindowId(9) });
    assert_eq!(focused(&world), Some(2));
}

#[test]
fn a_new_window_opening_takes_focus_to_the_strip() {
    let mut world = strip_of(1, 1);
    open(&mut world, 9);
    float_on_map(&mut world, 9);
    draw(&mut world, 9, 100, 100);
    open(&mut world, 2);
    assert_eq!(focused(&world), Some(2));
    assert!(placement(&world, 9).visible, "the floating window stays up");
}

/// A dialog whose parent is stacked under a fullscreen sibling in its
/// column: closing it must not hand focus to the parent (which would leave
/// the sibling fullscreen without being its column's focused window) --
/// nor end a fullscreen the dialog had nothing to do with.
#[test]
fn closing_a_dialog_does_not_refocus_a_parent_behind_a_fullscreen_sibling() {
    let mut world = strip_of(1, 1);
    open(&mut world, 2);
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    world.handle_action(Action::FocusWindow(Vertical::Up));
    assert_eq!(focused(&world), Some(1));
    world.handle_action(Action::ToggleFullscreen);
    assert!(world.is_fullscreen(WindowId(1)));
    open_with(&mut world, 9, with_parent(2));
    float_on_map(&mut world, 9);
    world.handle_event(Event::WindowClosed { id: WindowId(9) });
    assert!(
        world.is_fullscreen(WindowId(1)),
        "an unrelated fullscreen ended"
    );
    assert_eq!(focused(&world), Some(1));
    assert_eq!(world.fullscreen_on(OutputId(1)), Some(WindowId(1)));
}

/// A dialog alone on an inactive workspace, whose parent is on the next
/// one: closing it drops its workspace, and nothing may refocus the
/// neighbour that slid into its index.
#[test]
fn closing_a_lone_dialog_on_an_inactive_workspace_moves_no_other_focus() {
    let mut world = world();
    open(&mut world, 9);
    float_on_map(&mut world, 9);
    world.handle_action(Action::FocusWorkspace(Vertical::Down));
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::FocusWindowId(WindowId(2)));
    // Give the dialog a parent on the second workspace after the fact.
    world.handle_event(Event::WindowChanged {
        id: WindowId(9),
        info: with_parent(1),
    });
    world.handle_event(Event::WindowClosed { id: WindowId(9) });
    assert_eq!(focused(&world), Some(2));
}
