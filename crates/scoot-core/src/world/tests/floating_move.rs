//! Moving and resizing floating windows, and the order floating windows
//! are drawn in (a window's own dialogs above it).
//!
//! The screen is the shared 1000x600. Floating windows are placed within
//! the usable area itself (the gap is the strip's), so a window floated with
//! no parent and drawn 200x100 is centred at (400, 250).

use super::*;
use crate::{Action, Edges, Size, SizeHints};

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

/// A world with two tiled columns and floating window 9, drawn 200x100 and
/// focused, at (400, 250).
fn with_dialog() -> World {
    let mut world = world();
    for id in 1..=2 {
        open(&mut world, id);
        draw(&mut world, id, 485, 580);
    }
    open(&mut world, 9);
    float_on_map(&mut world, 9);
    draw(&mut world, 9, 200, 100);
    assert_eq!(placement(&world, 9).rect, Rect::new(400, 250, 200, 100));
    world
}

fn move_to(world: &mut World, id: u64, x: i32, y: i32) {
    world.handle_action(Action::MoveFloating {
        id: WindowId(id),
        x,
        y,
    });
}

fn resize(world: &mut World, id: u64, w: i32, h: i32, edges: Edges) {
    world.handle_action(Action::ResizeFloating {
        id: WindowId(id),
        size: Size::new(w, h),
        edges,
    });
}

const LEFT: Edges = Edges {
    left: true,
    right: false,
    top: false,
    bottom: false,
};
const TOP_LEFT: Edges = Edges {
    left: true,
    right: false,
    top: true,
    bottom: false,
};
const BOTTOM: Edges = Edges {
    left: false,
    right: false,
    top: false,
    bottom: true,
};

fn tiled(world: &World) -> Vec<(WindowId, Rect, bool)> {
    world
        .arrange()
        .placements
        .iter()
        .filter(|p| !p.floating)
        .map(|p| (p.id, p.rect, p.visible))
        .collect()
}

/// The ids of the visible placements, in drawing order.
fn drawn(world: &World) -> Vec<u64> {
    world
        .arrange()
        .placements
        .iter()
        .filter(|p| p.visible)
        .map(|p| p.id.0)
        .collect()
}

#[test]
fn a_move_puts_the_top_left_corner_where_asked_and_keeps_it() {
    let mut world = with_dialog();
    let strip = tiled(&world);
    move_to(&mut world, 9, 50, 60);
    assert_eq!(placement(&world, 9).rect, Rect::new(50, 60, 200, 100));
    assert_eq!(
        tiled(&world),
        strip,
        "a floating move never moves the strip"
    );
    assert_eq!(focused(&world), Some(9), "a move is not a focus change");
    // The geometry a shell reads between arrangements is the placement.
    let geometry = world.floating_geometry(WindowId(9)).expect("floating");
    assert_eq!(geometry.rect, placement(&world, 9).rect);
    assert_eq!(geometry.output, OutputId(1));
    assert_eq!(geometry.requested, None, "a move asks for no size");
    // Kept by its middle, as before the move: redrawing larger grows it
    // around the point it was moved to.
    draw(&mut world, 9, 240, 120);
    assert_eq!(placement(&world, 9).rect, Rect::new(30, 50, 240, 120));
}

#[test]
fn a_move_is_clamped_inside_the_usable_area() {
    let mut world = with_dialog();
    world.handle_event(Event::OutputUsableAreaChanged {
        id: OutputId(1),
        area: Rect::new(0, 30, 1000, 570),
    });
    move_to(&mut world, 9, -500, -500);
    assert_eq!(placement(&world, 9).rect, Rect::new(0, 30, 200, 100));
    move_to(&mut world, 9, i32::MAX, i32::MAX);
    assert_eq!(placement(&world, 9).rect, Rect::new(800, 500, 200, 100));
    move_to(&mut world, 9, i32::MIN, i32::MAX);
    assert_eq!(placement(&world, 9).rect, Rect::new(0, 500, 200, 100));
}

#[test]
fn moving_onto_another_output_carries_the_window_and_its_focus_there() {
    let mut world = with_dialog();
    world.handle_event(Event::OutputAdded {
        id: OutputId(2),
        area: Rect::new(1000, 0, 1920, 1080),
    });
    let strip = tiled(&world);
    // Its middle lands on the second output.
    move_to(&mut world, 9, 1500, 400);
    let placed = placement(&world, 9);
    assert_eq!(placed.output, OutputId(2));
    assert_eq!(placed.rect, Rect::new(1500, 400, 200, 100));
    assert!(placed.visible);
    assert_eq!(focused(&world), Some(9), "focus goes with it");
    assert_eq!(world.focused_output(), Some(OutputId(2)));
    assert_eq!(tiled(&world), strip, "neither strip moved");
    // Straddling the edge with its middle still on the first output: back
    // there, and clamped inside it.
    move_to(&mut world, 9, 850, 100);
    let placed = placement(&world, 9);
    assert_eq!(placed.output, OutputId(1));
    assert_eq!(placed.rect, Rect::new(800, 100, 200, 100));
    assert_eq!(world.focused_output(), Some(OutputId(1)));
}

#[test]
fn a_move_whose_middle_is_over_no_output_stays_on_its_own() {
    let mut world = with_dialog();
    world.handle_event(Event::OutputAdded {
        id: OutputId(2),
        area: Rect::new(1000, 0, 1920, 1080),
    });
    // Below the first output, left of the second: over nothing.
    move_to(&mut world, 9, 300, 900);
    let placed = placement(&world, 9);
    assert_eq!(placed.output, OutputId(1));
    assert_eq!(placed.rect, Rect::new(300, 500, 200, 100));
}

#[test]
fn moving_an_unfocused_floating_window_to_another_output_keeps_focus() {
    let mut world = with_dialog();
    world.handle_event(Event::OutputAdded {
        id: OutputId(2),
        area: Rect::new(1000, 0, 1920, 1080),
    });
    world.handle_action(Action::ToggleFloatingFocus);
    let focus = focused(&world);
    assert_ne!(focus, Some(9));
    move_to(&mut world, 9, 1500, 400);
    assert_eq!(placement(&world, 9).output, OutputId(2));
    assert!(
        placement(&world, 9).visible,
        "on the target's active workspace"
    );
    assert_eq!(focused(&world), focus);
    assert_eq!(world.focused_output(), Some(OutputId(1)));
}

#[test]
fn moves_and_resizes_ignore_tiled_fullscreen_and_unknown_windows() {
    let mut world = with_dialog();
    let before = world.arrange();
    move_to(&mut world, 1, 0, 0);
    resize(&mut world, 1, 100, 100, Edges::BOTTOM_RIGHT);
    move_to(&mut world, 77, 0, 0);
    resize(&mut world, 77, 100, 100, Edges::BOTTOM_RIGHT);
    assert_eq!(world.arrange(), before);
    assert_eq!(world.floating_geometry(WindowId(1)), None);
    world.handle_action(Action::SetFullscreen {
        id: WindowId(9),
        fullscreen: true,
    });
    let before = world.arrange();
    move_to(&mut world, 9, 0, 0);
    resize(&mut world, 9, 100, 100, Edges::BOTTOM_RIGHT);
    assert_eq!(world.arrange(), before);
    assert_eq!(world.floating_geometry(WindowId(9)), None);
}

#[test]
fn resizing_from_the_left_edge_keeps_the_right_edge_whatever_the_client_draws() {
    let mut world = with_dialog();
    let strip = tiled(&world);
    resize(&mut world, 9, 300, 100, LEFT);
    let asked = placement(&world, 9);
    assert_eq!(asked.requested, Some(Size::new(300, 100)));
    // Nothing moves until the window draws its new size...
    assert_eq!(asked.rect, Rect::new(400, 250, 200, 100));
    // ...and then it grows leftwards, the right edge where it was.
    draw(&mut world, 9, 300, 100);
    assert_eq!(placement(&world, 9).rect, Rect::new(300, 250, 300, 100));
    // A client rounding to its own step (a terminal's cells) still keeps
    // the right edge.
    draw(&mut world, 9, 291, 100);
    assert_eq!(placement(&world, 9).rect.right(), 600);
    assert_eq!(tiled(&world), strip);
}

#[test]
fn resizing_from_the_top_left_corner_keeps_the_bottom_right_corner() {
    let mut world = with_dialog();
    resize(&mut world, 9, 260, 180, TOP_LEFT);
    draw(&mut world, 9, 260, 180);
    let rect = placement(&world, 9).rect;
    assert_eq!((rect.right(), rect.bottom()), (600, 350));
    assert_eq!(rect.size(), Size::new(260, 180));
}

#[test]
fn a_resize_by_number_keeps_the_top_left_corner() {
    let mut world = with_dialog();
    resize(&mut world, 9, 300, 150, Edges::BOTTOM_RIGHT);
    draw(&mut world, 9, 300, 150);
    assert_eq!(placement(&world, 9).rect, Rect::new(400, 250, 300, 150));
}

#[test]
fn an_axis_with_no_edge_keeps_its_size() {
    let mut world = with_dialog();
    resize(&mut world, 9, 999, 160, BOTTOM);
    assert_eq!(placement(&world, 9).requested, Some(Size::new(200, 160)));
    draw(&mut world, 9, 200, 160);
    assert_eq!(placement(&world, 9).rect, Rect::new(400, 250, 200, 160));
}

#[test]
fn a_resize_respects_the_window_s_limits() {
    let mut world = with_dialog();
    let limits = |min: Size, max: Size| WindowInfo {
        hints: SizeHints { min, max },
        ..WindowInfo::default()
    };
    world.handle_event(Event::WindowChanged {
        id: WindowId(9),
        info: limits(Size::new(150, 80), Size::new(250, 120)),
    });
    resize(&mut world, 9, 900, 900, Edges::BOTTOM_RIGHT);
    assert_eq!(placement(&world, 9).requested, Some(Size::new(250, 120)));
    resize(&mut world, 9, 10, 10, Edges::BOTTOM_RIGHT);
    assert_eq!(placement(&world, 9).requested, Some(Size::new(150, 80)));
    // A maximum below the minimum: the minimum wins.
    world.handle_event(Event::WindowChanged {
        id: WindowId(9),
        info: limits(Size::new(150, 80), Size::new(100, 50)),
    });
    resize(&mut world, 9, 10, 10, Edges::BOTTOM_RIGHT);
    assert_eq!(placement(&world, 9).requested, Some(Size::new(150, 80)));
    // Nonsense in, at least 1 out.
    world.handle_event(Event::WindowChanged {
        id: WindowId(9),
        info: WindowInfo::default(),
    });
    resize(&mut world, 9, i32::MIN, 0, Edges::BOTTOM_RIGHT);
    assert_eq!(placement(&world, 9).requested, Some(Size::new(1, 1)));
}

#[test]
fn a_resize_stops_at_the_usable_area_instead_of_pushing_the_fixed_edge() {
    let mut world = with_dialog();
    // Top-left held at (400, 250): room for 600x350 inside 1000x600.
    resize(&mut world, 9, 5000, 5000, Edges::BOTTOM_RIGHT);
    assert_eq!(placement(&world, 9).requested, Some(Size::new(600, 350)));
    draw(&mut world, 9, 600, 350);
    assert_eq!(placement(&world, 9).rect, Rect::new(400, 250, 600, 350));
    // And from the left, the right edge (1000) held: room back to x = 0.
    resize(&mut world, 9, 5000, 350, LEFT);
    assert_eq!(placement(&world, 9).requested, Some(Size::new(1000, 350)));
}

#[test]
fn a_drag_of_resizes_holds_one_edge_even_when_the_client_draws_larger() {
    // A client with a minimum it never declared: asked for less, it draws
    // 250 wide, so it is placed shifted in from the left edge -- and the
    // next resize of the same drag must not take the shifted edge as the one
    // to hold.
    let mut world = with_dialog();
    move_to(&mut world, 9, 10, 250);
    resize(&mut world, 9, 150, 100, LEFT);
    let right = placement(&world, 9).rect.right();
    for width in [120, 100, 80, 60] {
        draw(&mut world, 9, 250, 100);
        resize(&mut world, 9, width, 100, LEFT);
    }
    draw(&mut world, 9, 60, 100);
    assert_eq!(placement(&world, 9).rect.right(), right);
}

#[test]
fn a_moved_window_is_re_centred_when_its_output_changes_size() {
    let mut world = with_dialog();
    move_to(&mut world, 9, 10, 10);
    world.handle_event(Event::OutputChanged {
        id: OutputId(1),
        area: Rect::new(0, 0, 2000, 1200),
    });
    // Back on the middle of the usable area -- which the output change kept
    // at 1000x600 until the platform reports it again (see `Output::set_area`).
    let rect = placement(&world, 9).rect;
    assert_eq!((rect.x + rect.w / 2, rect.y + rect.h / 2), (500, 300));
}

#[test]
fn clicking_a_floating_parent_keeps_its_dialog_drawn_above_it() {
    let mut world = world();
    open(&mut world, 1);
    draw(&mut world, 1, 980, 580);
    open(&mut world, 2);
    float_on_map(&mut world, 2);
    draw(&mut world, 2, 600, 400);
    open_with(
        &mut world,
        3,
        WindowInfo {
            parent: Some(WindowId(2)),
            ..WindowInfo::default()
        },
    );
    float_on_map(&mut world, 3);
    draw(&mut world, 3, 200, 100);
    assert_eq!(drawn(&world), vec![1, 2, 3]);
    // The parent is clicked: it is focused, and on top of the stack...
    world.handle_event(Event::FocusObserved { id: WindowId(2) });
    assert_eq!(focused(&world), Some(2));
    // ...but its dialog is still drawn over it.
    assert_eq!(drawn(&world), vec![1, 2, 3]);
}

/// The PR #242 re-review case: a floating fullscreen game, its dialog, and
/// an unrelated floating window. The game is clicked (raised above its
/// dialog), then the other window takes focus: the game shows behind it,
/// and its dialog must show too, above the game.
#[test]
fn a_fullscreen_game_behind_another_window_keeps_its_dialog_above_it() {
    let mut world = world();
    open(&mut world, 1);
    draw(&mut world, 1, 980, 580);
    open(&mut world, 7);
    float_on_map(&mut world, 7);
    draw(&mut world, 7, 300, 200);
    world.handle_action(Action::ToggleFullscreen);
    open_with(
        &mut world,
        8,
        WindowInfo {
            parent: Some(WindowId(7)),
            ..WindowInfo::default()
        },
    );
    float_on_map(&mut world, 8);
    draw(&mut world, 8, 200, 100);
    open(&mut world, 9);
    float_on_map(&mut world, 9);
    draw(&mut world, 9, 150, 100);
    world.handle_event(Event::FocusObserved { id: WindowId(7) });
    world.handle_event(Event::FocusObserved { id: WindowId(9) });
    assert_eq!(focused(&world), Some(9));
    let game = placement(&world, 7);
    assert!(game.visible && game.fullscreen, "{game:?}");
    assert!(
        placement(&world, 8).visible,
        "the dialog is hidden under the game"
    );
    assert_eq!(drawn(&world), vec![7, 8, 9]);
}

#[test]
fn a_dialog_of_a_dialog_stays_above_the_first() {
    let mut world = world();
    let child = |parent: u64| WindowInfo {
        parent: Some(WindowId(parent)),
        ..WindowInfo::default()
    };
    open(&mut world, 1);
    float_on_map(&mut world, 1);
    draw(&mut world, 1, 700, 500);
    open_with(&mut world, 2, child(1));
    float_on_map(&mut world, 2);
    draw(&mut world, 2, 400, 300);
    open_with(&mut world, 3, child(2));
    float_on_map(&mut world, 3);
    draw(&mut world, 3, 200, 100);
    // The middle dialog clicked, then the app: stack [3, 2, 1].
    world.handle_event(Event::FocusObserved { id: WindowId(2) });
    world.handle_event(Event::FocusObserved { id: WindowId(1) });
    assert_eq!(focused(&world), Some(1));
    assert_eq!(drawn(&world), vec![1, 2, 3]);
}

#[test]
fn windows_parented_in_a_loop_are_each_drawn_once() {
    let mut world = world();
    let child = |parent: u64| WindowInfo {
        parent: Some(WindowId(parent)),
        ..WindowInfo::default()
    };
    for id in 1..=3 {
        open(&mut world, id);
        float_on_map(&mut world, id);
        draw(&mut world, id, 100, 100);
    }
    for (id, parent) in [(1, 3), (2, 1), (3, 2)] {
        world.handle_event(Event::WindowChanged {
            id: WindowId(id),
            info: child(parent),
        });
    }
    let mut ids = drawn(&world);
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2, 3]);
}
