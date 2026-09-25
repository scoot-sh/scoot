//! The toggle both ways, focus and stacking, travel, and fullscreen from a
//! floating window -- through what the client is told and where it lands.

use scoot_core::{Action, Vertical};

use super::*;

/// Floating a column tells the client it chooses its own size and is not
/// tiled; un-floating tells it its column's size and that it is tiled again,
/// and puts it back where it was in the strip.
#[test]
fn the_toggle_flips_the_tiled_states_and_the_size_both_ways() {
    let mut fixture = Fixture::new();
    fixture.map(Spec::tiled());
    let window = fixture.map(Spec {
        color: DIALOG_BGRA,
        ..Spec::tiled()
    });
    let column = fixture.placement(window).rect;
    let order = fixture.strip();

    fixture.act(Action::ToggleFloating);
    assert!(fixture.floating(window));
    let floated = fixture.last_configure(window);
    assert_eq!((floated.width, floated.height), (0, 0), "{floated:?}");
    assert!(!floated.any_tiled, "{floated:?}");
    // Until it draws at the size it chose it keeps the one it has, centred.
    let placed = fixture.placement(window);
    assert!(placed.visible);
    assert_eq!((placed.rect.w, placed.rect.h), (column.w, column.h));
    fixture.done(Step::Draw { window });
    let placed = fixture.placement(window);
    assert_eq!(placed.rect, Rect::new(90, 100, NATURAL.0, NATURAL.1));

    fixture.act(Action::ToggleFloating);
    assert!(!fixture.floating(window));
    let tiled = fixture.last_configure(window);
    assert!(tiled.tiled, "{tiled:?}");
    assert_eq!((tiled.width, tiled.height), (column.w, column.h));
    fixture.done(Step::Draw { window });
    // Same columns in the same order at the same widths (the scroll may
    // differ: the strip was re-scrolled to the focused column).
    let widths = |strip: &[(WindowId, Rect)]| -> Vec<(WindowId, i32)> {
        strip.iter().map(|(id, rect)| (*id, rect.w)).collect()
    };
    assert_eq!(widths(&fixture.strip()), widths(&order));
    assert_eq!(fixture.state.focus, Some(fixture.id(window)));
}

#[test]
fn un_floating_by_id_goes_right_of_the_strip_focus_without_taking_focus() {
    let mut fixture = Fixture::new();
    for _ in 0..3 {
        fixture.map(Spec::tiled());
    }
    let (first, third) = (fixture.id(0), fixture.id(2));
    fixture.act(Action::SetFloating {
        id: third,
        floating: true,
    });
    fixture.act(Action::FocusWindowId(first));
    fixture.act(Action::SetFloating {
        id: third,
        floating: false,
    });
    let order: Vec<WindowId> = fixture.strip().into_iter().map(|(id, _)| id).collect();
    assert_eq!(order, vec![first, third, fixture.id(1)]);
    assert_eq!(fixture.state.focus, Some(first));
    assert!(fixture.last_configure(2).tiled);
}

/// Raise on focus, in pixels: the most recently focused of two overlapping
/// floating windows is the one drawn on top.
#[test]
fn focusing_a_floating_window_raises_it_over_the_other() {
    let mut fixture = Fixture::new();
    fixture.map(Spec::tiled());
    let lower = fixture.map(Spec {
        color: DIALOG_BGRA,
        dialog: true,
        ..Spec::tiled()
    });
    let upper = fixture.map(Spec {
        color: OTHER_BGRA,
        dialog: true,
        ..Spec::tiled()
    });
    let (x, y) = centre(fixture.placement(lower).rect);
    assert_eq!(centre(fixture.placement(upper).rect), (x, y), "overlapping");
    let pixels = fixture.render();
    assert_eq!(pixel(&pixels, x, y), OTHER_BGRA);
    fixture.act(Action::FocusWindowId(fixture.id(lower)));
    let pixels = fixture.render();
    assert_eq!(pixel(&pixels, x, y), DIALOG_BGRA);
    // A click lands on what is drawn: the raised window, and it raises the
    // one clicked.
    fixture.click(f64::from(x), f64::from(y));
    assert_eq!(fixture.state.focus, Some(fixture.id(lower)));
}

#[test]
fn the_focus_toggle_hands_the_keyboard_between_the_layers() {
    let mut fixture = Fixture::new();
    fixture.map(Spec::tiled());
    let dialog = fixture.map(Spec::dialog_of(0));
    let surface = |fixture: &Fixture, index: usize| {
        fixture
            .state
            .windows
            .get(&fixture.id(index))
            .and_then(smithay::desktop::Window::toplevel)
            .map(|toplevel| toplevel.wl_surface().clone())
    };
    let keyboard = |fixture: &Fixture| {
        fixture
            .state
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
    };
    assert_eq!(keyboard(&fixture), surface(&fixture, dialog));
    fixture.act(Action::ToggleFloatingFocus);
    assert_eq!(keyboard(&fixture), surface(&fixture, 0));
    fixture.act(Action::ToggleFloatingFocus);
    assert_eq!(keyboard(&fixture), surface(&fixture, dialog));
}

#[test]
fn a_floating_window_moved_to_another_workspace_stays_floating() {
    let mut fixture = Fixture::new();
    fixture.map(Spec::tiled());
    let dialog = fixture.map(Spec::dialog_of(0));
    let rect = fixture.placement(dialog).rect;
    fixture.act(Action::MoveWindowToWorkspace(Vertical::Down));
    let placed = fixture.placement(dialog);
    assert!(placed.floating && placed.visible);
    assert_eq!(placed.rect, rect);
    assert!(!fixture.placement(0).visible, "the old workspace is away");
    assert!(!fixture.last_configure(dialog).any_tiled);
}

/// A floating window can go fullscreen -- told the output's size and the
/// `fullscreen` state -- and leaving puts it back floating: told it chooses
/// its size again, with no tiled state, and placed where it was.
#[test]
fn a_floating_window_goes_fullscreen_and_comes_back_floating() {
    let mut fixture = Fixture::new();
    fixture.map(Spec::tiled());
    let dialog = fixture.map(Spec::dialog_of(0));
    let floated = fixture.placement(dialog);

    fixture.done(Step::SetFullscreen { window: dialog });
    let entered = fixture.last_configure(dialog);
    assert!(entered.fullscreen && !entered.any_tiled, "{entered:?}");
    assert_eq!((entered.width, entered.height), (CANVAS, CANVAS));
    fixture.done(Step::Draw { window: dialog });
    assert_eq!(
        fixture.placement(dialog).rect,
        Rect::new(0, 0, CANVAS, CANVAS)
    );
    assert_eq!(
        fixture.state.world.fullscreen_on(scoot_core::OutputId(1)),
        Some(fixture.id(dialog))
    );
    let pixels = fixture.render();
    assert_eq!(pixel(&pixels, 2, 2), DIALOG_BGRA, "edge to edge");

    fixture.done(Step::UnsetFullscreen { window: dialog });
    let left = fixture.last_configure(dialog);
    assert!(!left.fullscreen && !left.any_tiled, "{left:?}");
    assert_eq!((left.width, left.height), (0, 0), "{left:?}");
    fixture.done(Step::Draw { window: dialog });
    assert!(fixture.floating(dialog));
    assert_eq!(fixture.placement(dialog), floated);
}
