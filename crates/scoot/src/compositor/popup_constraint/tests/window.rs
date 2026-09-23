//! A window's popups: flipped, slid or resized into the window's output --
//! its usable area, or the whole output while the window covers it
//! fullscreen -- and left exactly where the positioner put them when they
//! asked for no adjustment.

use super::*;

/// A 60x40 menu opened 10px left of the output's right edge, growing right:
/// 50px of it would be off the output.
fn menu_near_right_edge(fixture: &Fixture, window: usize) -> Spec {
    let rect = fixture.rect_of(window);
    let right = fixture.output_rect(1).right() - rect.x;
    Spec::menu_at(right - 10, 20, 60, 40)
}

#[test]
fn a_menu_crossing_the_right_edge_flips_to_the_anchors_other_side() {
    let mut fixture = Fixture::one_output();
    let window = fixture.window();
    let spec = menu_near_right_edge(&fixture, window).adjust(Adjust::FlipX);
    let (ax, ..) = spec.anchor_rect;

    let geometry = fixture.popup(Parent::Window(window), spec);

    // Flipped: anchored at the 1x1 anchor rect's right edge, growing left.
    assert_eq!(geometry, (ax + 1 - 60, 20, 60, 40));
}

#[test]
fn a_menu_crossing_the_right_edge_slides_back_inside() {
    let mut fixture = Fixture::one_output();
    let window = fixture.window();
    let rect = fixture.rect_of(window);
    let spec = menu_near_right_edge(&fixture, window).adjust(Adjust::SlideX);

    let geometry = fixture.popup(Parent::Window(window), spec);

    // Its right edge on the output's right edge.
    assert_eq!(geometry, (CANVAS - rect.x - 60, 20, 60, 40));
}

#[test]
fn a_menu_wider_than_the_output_is_resized_to_fit() {
    let mut fixture = Fixture::one_output();
    let window = fixture.window();
    let rect = fixture.rect_of(window);
    // 300 wide from 5px in from the output's left edge.
    let spec = Spec::menu_at(5 - rect.x, 20, 300, 40).adjust(Adjust::ResizeX);

    let geometry = fixture.popup(Parent::Window(window), spec);

    assert_eq!(geometry, (5 - rect.x, 20, CANVAS - 5, 40));
}

/// Without a flag for an axis, the protocol says the compositor "will
/// assume that the child surface should not change its position on that
/// axis when constrained" -- so it is cut, as before. (The shared-edge
/// counterpart is `output_clip`'s `a_popup_crossing_the_shared_edge_is_cut_there`,
/// which asks for no adjustment either and still passes unchanged.)
#[test]
fn a_menu_without_adjustment_flags_is_left_where_it_asked_to_be() {
    let mut fixture = Fixture::one_output();
    let window = fixture.window();
    let spec = menu_near_right_edge(&fixture, window);
    let (ax, ay, ..) = spec.anchor_rect;

    let geometry = fixture.popup(Parent::Window(window), spec);

    assert_eq!(geometry, (ax, ay, 60, 40));
}

/// Flags for one axis do nothing on the other: a menu crossing the right
/// edge that may only slide vertically stays cut on the right.
#[test]
fn an_adjustment_only_applies_on_its_own_axis() {
    let mut fixture = Fixture::one_output();
    let window = fixture.window();
    let spec = menu_near_right_edge(&fixture, window).adjust(Adjust::SlideY | Adjust::FlipY);
    let (ax, ay, ..) = spec.anchor_rect;

    let geometry = fixture.popup(Parent::Window(window), spec);

    assert_eq!(geometry, (ax, ay, 60, 40));
}

/// The shared edge between two outputs is an edge like any other: a menu
/// on the first output's window that would cross onto the second slides
/// back onto its own output -- and is drawn there, whole, with nothing of it
/// on the neighbour.
#[test]
fn a_menu_at_a_shared_edge_stays_on_its_parents_output() {
    let mut fixture = Fixture::two_outputs();
    fixture.pointer_on(1);
    let window = fixture.window();
    let rect = fixture.rect_of(window);
    assert!(
        rect.right() <= CANVAS,
        "the window is on the first output: {rect:?}"
    );
    let spec = menu_near_right_edge(&fixture, window).adjust(Adjust::SlideX);

    let geometry = fixture.popup(Parent::Window(window), spec);
    assert_eq!(geometry, (CANVAS - rect.x - 60, 20, 60, 40));

    let row = rect.y + 20 + 20;
    let first = fixture.frame(1);
    assert_eq!(pixel(&first, CANVAS - 60, row), POPUP_BGRA, "its left edge");
    assert_eq!(pixel(&first, CANVAS - 1, row), POPUP_BGRA, "its right edge");
    let second = fixture.frame(2);
    assert!(
        !test_support::contains(&second, POPUP_BGRA),
        "the menu was drawn on the other output"
    );
}

/// ...and from the other side: a menu on the second output's window growing
/// left over the shared edge slides right, onto the second output. Its
/// geometry stays relative to its parent, so this also checks the target is
/// the parent's output in the parent's coordinates, not the first output's.
#[test]
fn a_menu_on_the_second_output_slides_off_the_shared_edge_to_the_right() {
    let mut fixture = Fixture::two_outputs();
    fixture.pointer_on(2);
    let window = fixture.window();
    let rect = fixture.rect_of(window);
    assert_eq!(
        rect.x,
        CANVAS + GAP,
        "the window is on the second output: {rect:?}"
    );
    // Grows left from the parent's own left edge, 60 wide.
    let spec = Spec {
        gravity: Gravity::BottomLeft,
        ..Spec::menu_at(0, 20, 60, 40)
    }
    .adjust(Adjust::SlideX);

    let geometry = fixture.popup(Parent::Window(window), spec);

    // Its left edge on the second output's left edge.
    assert_eq!(geometry, (CANVAS - rect.x, 20, 60, 40));
    let second = fixture.frame(2);
    assert_eq!(pixel(&second, 0, rect.y + 40), POPUP_BGRA);
    let first = fixture.frame(1);
    assert!(!test_support::contains(&first, POPUP_BGRA));
}

/// A window's popups are drawn below a top-layer bar, so the bar's
/// exclusive zone is not somewhere a menu can be slid to: it would be hidden
/// under the bar. The target is the output's usable area.
#[test]
fn a_menu_slides_below_a_top_bar_not_under_it() {
    let mut fixture = Fixture::one_output();
    fixture.done(Step::MapBar {
        output: 0,
        height: 30,
        exclusive: 30,
    });
    let window = fixture.window();
    let rect = fixture.rect_of(window);
    assert_eq!(rect.y, 30 + GAP, "the window tiles below the bar: {rect:?}");
    // 40 tall, growing *up* from the window's own top edge: its top would
    // be at y = 2, inside the output but under the bar.
    let spec = Spec {
        gravity: Gravity::TopRight,
        ..Spec::menu_at(10, 0, 60, 40)
    }
    .adjust(Adjust::SlideY);

    let geometry = fixture.popup(Parent::Window(window), spec);

    assert_eq!(
        geometry,
        (10, 30 - rect.y, 60, 40),
        "its top on the bar's bottom edge"
    );
}

/// While a window covers its output fullscreen, the top layer is not drawn
/// at all, so the whole output is the menu's to use -- the bar's zone
/// included.
#[test]
fn a_fullscreen_windows_menu_may_use_the_whole_output() {
    let mut fixture = Fixture::one_output();
    fixture.done(Step::MapBar {
        output: 0,
        height: 30,
        exclusive: 30,
    });
    let window = fixture.window();
    fixture.done(Step::Fullscreen { window });
    assert_eq!(fixture.rect_of(window), fixture.output_rect(1));
    assert_eq!(
        fixture.state.world.fullscreen_on(OutputId(1)),
        Some(fixture.id(window))
    );
    // Growing up from y = 5: its top would be at -35, off the output.
    let spec = Spec {
        gravity: Gravity::TopRight,
        ..Spec::menu_at(10, 5, 60, 40)
    }
    .adjust(Adjust::SlideY);

    let geometry = fixture.popup(Parent::Window(window), spec);

    assert_eq!(
        geometry,
        (10, 0, 60, 40),
        "slid to the output's top, over the bar's zone"
    );
}

/// A submenu is positioned relative to its parent *menu*, so its target
/// must be too. The numbers are chosen so that measuring the target from
/// the window instead of from the parent menu would leave the submenu
/// unconstrained, and unflipped.
#[test]
fn a_submenu_flips_against_the_edge_measured_from_its_parent_menu() {
    let mut fixture = Fixture::one_output();
    let window = fixture.window();
    let rect = fixture.rect_of(window);
    // The parent menu: 60 wide, its left edge at global x = 100.
    let menu = fixture.popup(
        Parent::Window(window),
        Spec::menu_at(100 - rect.x, 20, 60, 80),
    );
    assert_eq!(menu, (100 - rect.x, 20, 60, 80));
    // The submenu opens off the parent menu's right edge (global 160),
    // 60 wide: it would reach global 220, 20px past the output's edge.
    // Measured from the window instead, its right edge (120 in the parent
    // menu's coordinates) would look well inside `CANVAS - rect.x`.
    let submenu = Spec {
        size: (60, 40),
        anchor_rect: (0, 10, 60, 1),
        anchor: Anchor::TopRight,
        gravity: Gravity::BottomRight,
        offset: (0, 0),
        adjust: Adjust::FlipX,
    };

    let geometry = fixture.popup(Parent::Popup(0), submenu);

    // Flipped to open off the parent menu's left edge instead.
    assert_eq!(geometry, (-60, 10, 60, 40));
}

/// `xdg_popup.reposition` is constrained exactly like the initial
/// configure, and still echoes the client's token.
#[test]
fn a_reposition_is_constrained_too() {
    let mut fixture = Fixture::one_output();
    let window = fixture.window();
    let rect = fixture.rect_of(window);
    let inside = fixture.popup(Parent::Window(window), Spec::menu_at(10, 20, 60, 40));
    assert_eq!(inside, (10, 20, 60, 40));

    let spec = menu_near_right_edge(&fixture, window).adjust(Adjust::SlideX);
    let Ack::Repositioned { geometry, token } = fixture.run(Step::Reposition {
        popup: 0,
        spec,
        token: 7,
    }) else {
        panic!("expected a repositioned popup");
    };

    assert_eq!(token, 7);
    assert_eq!(geometry, (CANVAS - rect.x - 60, 20, 60, 40));
}
