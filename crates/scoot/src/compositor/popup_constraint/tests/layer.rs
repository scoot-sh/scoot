//! A layer surface's popups -- a bar's dropdown -- are constrained against
//! the layer surface's whole output, in the layer surface's own coordinates.

use super::*;

/// A 60x40 dropdown opened in a full-width top bar 10px left of the
/// output's right edge, growing right.
fn dropdown_near_right_edge() -> Spec {
    Spec::menu_at(CANVAS - 10, 20, 60, 40).adjust(Adjust::FlipX)
}

fn bar(output: usize) -> Step {
    Step::MapBar {
        output,
        height: 30,
        exclusive: 30,
    }
}

#[test]
fn a_bars_dropdown_crossing_the_right_edge_flips() {
    let mut fixture = Fixture::one_output();
    fixture.done(bar(0));

    let geometry = fixture.popup(Parent::Layer(0), dropdown_near_right_edge());

    assert_eq!(geometry, (CANVAS - 10 + 1 - 60, 20, 60, 40));
}

/// The bar lives in the zone it reserved, and its dropdown is drawn with it
/// at its own layer, so the usable area would be the wrong target: a
/// dropdown overlapping its own bar is not pushed out of it.
#[test]
fn a_bars_dropdown_is_not_pushed_out_of_the_bars_own_zone() {
    let mut fixture = Fixture::one_output();
    fixture.done(bar(0));
    // y = 5..45: across the bar's 30px zone, well inside the output.
    let spec = Spec::menu_at(10, 5, 60, 40).adjust(Adjust::SlideY);

    let geometry = fixture.popup(Parent::Layer(0), spec);

    assert_eq!(geometry, (10, 5, 60, 40));
}

/// On the second output: the target is that output, in the bar's own
/// coordinates -- not the first output's, and not global coordinates.
#[test]
fn a_bars_dropdown_on_the_second_output_flips_against_that_outputs_edge() {
    let mut fixture = Fixture::two_outputs();
    fixture.done(bar(1));

    let geometry = fixture.popup(Parent::Layer(0), dropdown_near_right_edge());

    let x = CANVAS - 10 + 1 - 60;
    assert_eq!(geometry, (x, 20, 60, 40));
    let second = fixture.frame(2);
    assert_eq!(pixel(&second, x, 30), POPUP_BGRA);
    assert_eq!(pixel(&second, x + 59, 30), POPUP_BGRA);
    let first = fixture.frame(1);
    assert!(!test_support::contains(&first, POPUP_BGRA));
}
