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
