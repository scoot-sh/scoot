//! The frame-list half of the primary-direct rule, over `(is_rounded,
//! alpha)` pairs.
//!
//! The whole rule -- lock, covering fullscreen window, and this half over a
//! list a real `State` gathered -- is driven through a live client in
//! `fullscreen/tests/primary_direct.rs`, which runs the same
//! `scanout_frame_elements` the tier runs. These pin the combinations a
//! live scene cannot build on demand: a rounded element on a covered output
//! (which `window_elements` never produces for the covering window itself),
//! and alpha values at the boundary.

use super::*;

/// An opaque, unrounded element: what a covering window's surface is.
const PLAIN: (bool, f32) = (false, 1.0);

#[test]
fn an_all_opaque_unrounded_frame_is_eligible() {
    assert_eq!(judge_elements([PLAIN]), PrimaryDirect::Eligible);
    assert_eq!(
        judge_elements([PLAIN, PLAIN, PLAIN]),
        PrimaryDirect::Eligible
    );
    // A covered output whose frame gathered nothing: nothing to scan out,
    // and nothing that could be wrongly scanned out either. Smithay has no
    // element to try and composites the clear colour.
    assert_eq!(judge_elements([]), PrimaryDirect::Eligible);
}

#[test]
fn a_rounded_element_anywhere_refuses() {
    // `Rounded` forwards the unclipped buffer as its underlying storage, so
    // one reaching the primary would lose its corners. Refused wherever it
    // sits in the list: which element Smithay would try is its own call.
    let rounded = (true, 1.0);
    for list in [
        vec![rounded],
        vec![rounded, PLAIN],
        vec![PLAIN, rounded],
        vec![PLAIN, PLAIN, rounded],
    ] {
        assert_eq!(judge_elements(list), PrimaryDirect::Rounded);
    }
}

#[test]
fn a_translucent_element_anywhere_refuses() {
    for alpha in [0.0, 0.5, 0.999_999] {
        for list in [vec![(false, alpha)], vec![PLAIN, (false, alpha)]] {
            assert_eq!(
                judge_elements(list),
                PrimaryDirect::Translucent,
                "alpha {alpha}"
            );
        }
    }
}

#[test]
fn a_fully_opaque_alpha_modifier_is_exactly_one() {
    // What Smithay's `from_surface` multiplies in for a surface whose client
    // set the multiplier to fully opaque (`multiplier_f32`): it must compare
    // as opaque, or a client that sets `u32::MAX` explicitly would never go
    // direct while an identical client that never touched the protocol
    // would.
    let opaque = u32::MAX as f32 / u32::MAX as f32;
    assert_eq!(judge_elements([(false, opaque)]), PrimaryDirect::Eligible);
    // And a multiplier just below it (one f32 step) is translucent, not
    // rounded up to opaque.
    let almost = (u32::MAX - (1 << 8)) as f32 / u32::MAX as f32;
    assert!(almost < 1.0, "the test's own premise");
    assert_eq!(
        judge_elements([(false, almost)]),
        PrimaryDirect::Translucent
    );
}

#[test]
fn only_eligible_is_allowed() {
    for refusal in [
        PrimaryDirect::Locked,
        PrimaryDirect::NotCovered,
        PrimaryDirect::Streaming,
        PrimaryDirect::Translucent,
        PrimaryDirect::Rounded,
    ] {
        assert!(!refusal.allowed(), "{refusal:?}");
    }
    assert!(PrimaryDirect::Eligible.allowed());
}

// Rule 6: would Smithay try the primary at all.

use smithay::backend::renderer::Color32F;
use smithay::utils::{Physical, Rectangle};

const OUTPUT: (i32, i32) = (200, 100);
const GREY: Color32F = Color32F::new(0.08, 0.08, 0.1, 1.0);

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
    Rectangle::new((x, y).into(), (w, h).into())
}

/// A covering element: geometry the whole output, with `opaque` regions.
fn covering(
    opaque: Vec<Rectangle<i32, Physical>>,
) -> (Rectangle<i32, Physical>, Vec<Rectangle<i32, Physical>>) {
    (rect(0, 0, OUTPUT.0, OUTPUT.1), opaque)
}

#[test]
fn an_opaque_covering_element_is_tried_over_any_background() {
    // An `XR24` buffer, or an alpha buffer with a whole opaque region: one
    // region, the whole output.
    let whole = covering(vec![rect(0, 0, OUTPUT.0, OUTPUT.1)]);
    assert!(primary_can_be_tried(GREY, OUTPUT, [whole]));
    // Opaque in two pieces that together cover it.
    let halves = covering(vec![rect(0, 0, 100, 100), rect(100, 0, 100, 100)]);
    assert!(primary_can_be_tried(GREY, OUTPUT, [halves]));
    // Larger than the output: clipped to it, still covering.
    let larger = (rect(-5, -5, 210, 110), vec![rect(0, 0, 400, 400)]);
    assert!(primary_can_be_tried(GREY, OUTPUT, [larger]));
}

#[test]
fn an_alpha_buffer_with_no_opaque_region_is_not_tried_over_a_grey_background() {
    // The measured case: `AR24` and no opaque region. Smithay would not try
    // the primary, so neither the direct flags nor the scanout tranche are
    // of any use to it.
    assert!(!primary_can_be_tried(GREY, OUTPUT, [covering(Vec::new())]));
    // Opaque over only part of it.
    let part = covering(vec![rect(0, 0, 100, 100)]);
    assert!(!primary_can_be_tried(GREY, OUTPUT, [part]));
    let gap = covering(vec![rect(0, 0, 99, 100), rect(100, 0, 100, 100)]);
    assert!(!primary_can_be_tried(GREY, OUTPUT, [gap]));
    // Opaque, but smaller than the output (a not-yet-resized buffer).
    let small = (rect(0, 0, 150, 100), vec![rect(0, 0, 150, 100)]);
    assert!(!primary_can_be_tried(GREY, OUTPUT, [small]));
    // An empty frame.
    assert!(!primary_can_be_tried(
        GREY,
        OUTPUT,
        std::iter::empty::<(Rectangle<i32, Physical>, Vec<Rectangle<i32, Physical>>)>()
    ));
}

#[test]
fn a_black_or_transparent_clear_colour_is_always_tried() {
    // Smithay's other arm: over black (or nothing) a translucent or small
    // bottom element shows exactly what scanning it out would.
    for clear in [
        Color32F::new(0.0, 0.0, 0.0, 1.0),
        Color32F::new(0.3, 0.2, 0.1, 0.0),
    ] {
        assert!(primary_can_be_tried(clear, OUTPUT, [covering(Vec::new())]));
        let small = (rect(0, 0, 10, 10), Vec::new());
        assert!(primary_can_be_tried(clear, OUTPUT, [small]));
    }
}

#[test]
fn any_opaque_covering_element_counts_wherever_it_sits() {
    // A cursor or notification above a covering window, a wallpaper below
    // it: the covering one is what Smithay's walk stops at.
    let cursor = (rect(10, 10, 24, 24), vec![rect(0, 0, 24, 24)]);
    let wallpaper = (rect(0, 0, OUTPUT.0, OUTPUT.1), Vec::new());
    let window = covering(vec![rect(0, 0, OUTPUT.0, OUTPUT.1)]);
    assert!(primary_can_be_tried(
        GREY,
        OUTPUT,
        [cursor.clone(), window, wallpaper.clone()]
    ));
    assert!(!primary_can_be_tried(GREY, OUTPUT, [cursor, wallpaper]));
}
