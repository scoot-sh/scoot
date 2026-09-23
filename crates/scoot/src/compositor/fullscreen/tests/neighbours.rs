//! A fullscreen window its column is focused away from sits in the strip
//! exactly where a tiled column of the output's width would: an ordinary
//! gap from each neighbour, never over the focused window, and never taking
//! its clicks.
//!
//! The numbers, on the 200-square canvas with the default 12px gap and
//! half-width columns: the gap-inset usable area is `12..188` (176 wide), a
//! column is 82 wide, so two windows sit at `12..94` and `106..188`. With a
//! 20px dock reserving the left edge, the usable area is `32..188` (156
//! wide), a column is 72 wide, and the first window sits at `32..104`.

use scoot_core::{Action, Horizontal};

use super::*;

/// Maps two windows (`OTHER_BGRA`, then `WINDOW_BGRA`), makes the `full`-th
/// fullscreen (drawn at its fullscreen size), then focuses the other one.
fn focused_away(fixture: &mut Fixture, full: usize) {
    fixture.map(OTHER_BGRA);
    fixture.map(WINDOW_BGRA);
    fixture.state.act(Action::FocusWindowId(fixture.id(full)));
    fixture.settle();
    fixture.configured(Step::SetFullscreen {
        window: full,
        output: None,
    });
    let color = if full == 0 { OTHER_BGRA } else { WINDOW_BGRA };
    fixture.done(Step::Draw {
        window: full,
        color,
    });
    let other = 1 - full;
    fixture.state.act(Action::FocusWindowId(fixture.id(other)));
    fixture.settle();
    assert!(fixture.state.world.is_fullscreen(fixture.id(full)));
    assert_eq!(fixture.state.focus, Some(fixture.id(other)));
}

#[test]
fn a_fullscreen_right_neighbour_keeps_one_gap_from_the_focused_window() {
    let mut fixture = Fixture::new();
    focused_away(&mut fixture, 1);
    let focused = fixture.rect_of(0);
    let full = fixture.rect_of(1);
    assert_eq!(focused, Rect::new(GAP, GAP, 82, CANVAS - 2 * GAP));
    assert_eq!(full.x, focused.right() + GAP, "{full:?}");
    assert_eq!(full.size(), OUTPUT.size());

    // The gap is drawn as a gap: background between the two, the focused
    // window's own pixels up to its edge, the fullscreen one's after it.
    let pixels = fixture.render();
    assert_eq!(pixel(&pixels, focused.right() - 1, 100), OTHER_BGRA);
    assert_eq!(pixel(&pixels, full.x + 1, 100), WINDOW_BGRA);
}

#[test]
fn a_fullscreen_left_neighbour_keeps_one_gap_from_the_focused_window() {
    let mut fixture = Fixture::new();
    focused_away(&mut fixture, 0);
    let focused = fixture.rect_of(1);
    let full = fixture.rect_of(0);
    assert_eq!(full.right() + GAP, focused.x, "{full:?} vs {focused:?}");
    assert_eq!(full.size(), OUTPUT.size());
}

#[test]
fn a_left_exclusive_zone_does_not_push_it_over_the_focused_window() {
    let mut fixture = Fixture::new();
    fixture.done(Step::CreateLayer(Layer::Dock));
    focused_away(&mut fixture, 1);
    let focused = fixture.rect_of(0);
    let full = fixture.rect_of(1);
    let dock = DOCK_WIDTH as i32;
    assert_eq!(focused.x, dock + GAP, "{focused:?}");
    assert_eq!(full.x, focused.right() + GAP, "{full:?}");

    // Inside the focused window's right edge: its pixels, and its click.
    let inside = (focused.right() - 4, 100);
    let pixels = fixture.render();
    assert_eq!(
        pixel(&pixels, inside.0, inside.1),
        OTHER_BGRA,
        "the fullscreen window was drawn over the focused one"
    );
    fixture.click(f64::from(inside.0), f64::from(inside.1));
    assert_eq!(
        fixture.pointer(),
        Some(Entered::Window(0)),
        "a click on the focused window went to the fullscreen one"
    );
    assert_eq!(fixture.state.focus, Some(fixture.id(0)));

    // And focusing back covers the output again, dock hidden.
    fixture.state.act(Action::FocusColumn(Horizontal::Right));
    fixture.settle();
    assert_eq!(fixture.rect_of(1), OUTPUT);
}
