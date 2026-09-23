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

// Rule 6: Smithay's walk, its guard, and whose element it would try.

use smithay::backend::renderer::Color32F;
use smithay::utils::{Physical, Rectangle};

const OUTPUT: (i32, i32) = (200, 100);
const GREY: Color32F = Color32F::new(0.08, 0.08, 0.1, 1.0);
const BLACK: Color32F = Color32F::new(0.0, 0.0, 0.0, 1.0);

type Entry = (Rectangle<i32, Physical>, Vec<Rectangle<i32, Physical>>);

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
    Rectangle::new((x, y).into(), (w, h).into())
}

/// An element over the whole output with `opaque` regions.
fn full(opaque: Vec<Rectangle<i32, Physical>>) -> Entry {
    (rect(0, 0, OUTPUT.0, OUTPUT.1), opaque)
}

fn whole() -> Vec<Rectangle<i32, Physical>> {
    vec![rect(0, 0, OUTPUT.0, OUTPUT.1)]
}

fn walk(elements: Vec<Entry>) -> Option<WalkEnd> {
    smithay_walk(elements, OUTPUT, &mut JudgeScratch::default())
}

/// Rule 6 with the element at `window` being the covering window's.
fn decide(elements: Vec<Entry>, clear: Color32F, window: usize) -> PrimaryDirect {
    rule6(walk(elements), clear, |index| index == window)
}

#[test]
fn the_walk_stops_at_the_first_opaque_covering_element() {
    // Cursor, window (opaque), wallpaper: the window ends the list and the
    // wallpaper under it is never reached.
    let cursor = (rect(10, 10, 24, 24), vec![rect(0, 0, 24, 24)]);
    let end = walk(vec![cursor, full(whole()), full(whole())]);
    assert_eq!(
        end,
        Some(WalkEnd {
            index: 1,
            spans_opaque: true
        })
    );
    // Opaque in two pieces that together cover it: still opaque.
    let halves = full(vec![rect(0, 0, 100, 100), rect(100, 0, 100, 100)]);
    assert_eq!(
        walk(vec![halves]),
        Some(WalkEnd {
            index: 0,
            spans_opaque: true
        })
    );
    // Larger than the output: clipped to it, still covering.
    let larger = (rect(-5, -5, 210, 110), vec![rect(0, 0, 400, 400)]);
    assert_eq!(
        walk(vec![larger]),
        Some(WalkEnd {
            index: 0,
            spans_opaque: true
        })
    );
}

#[test]
fn without_an_opaque_covering_element_the_walk_ends_at_the_bottom_visible_one() {
    // An alpha window (no opaque region) over a wallpaper: the wallpaper is
    // the last element, and is what Smithay would try.
    let end = walk(vec![full(Vec::new()), full(Vec::new())]);
    assert_eq!(
        end,
        Some(WalkEnd {
            index: 1,
            spans_opaque: false
        })
    );
    // A partly opaque element above hides nothing wholly: still the bottom.
    let end = walk(vec![full(vec![rect(0, 0, 100, 100)]), full(Vec::new())]);
    assert_eq!(
        end,
        Some(WalkEnd {
            index: 1,
            spans_opaque: false
        })
    );
    // Two partly opaque elements that together hide the bottom one: it is
    // skipped, and the second is last.
    let end = walk(vec![
        full(vec![rect(0, 0, 100, 100)]),
        full(vec![rect(100, 0, 100, 100)]),
        full(Vec::new()),
    ]);
    assert_eq!(
        end,
        Some(WalkEnd {
            index: 1,
            spans_opaque: false
        })
    );
    // Elements off the output are never on the list; nothing at all is None.
    assert_eq!(walk(vec![(rect(300, 300, 10, 10), Vec::new())]), None);
    assert_eq!(walk(Vec::new()), None);
}

#[test]
fn an_opaque_covering_window_is_eligible_over_any_background() {
    for clear in [GREY, BLACK] {
        assert_eq!(
            decide(vec![full(whole())], clear, 0),
            PrimaryDirect::Eligible
        );
        // Over a wallpaper: the walk stops at the window.
        assert_eq!(
            decide(vec![full(whole()), full(whole())], clear, 0),
            PrimaryDirect::Eligible
        );
    }
}

#[test]
fn an_alpha_window_with_no_opaque_region_is_not_tried_over_grey() {
    assert_eq!(
        decide(vec![full(Vec::new())], GREY, 0),
        PrimaryDirect::NothingOpaqueCovers
    );
    // Opaque over part of it, or smaller than the output.
    assert_eq!(
        decide(
            vec![full(vec![rect(0, 0, 99, 100), rect(100, 0, 100, 100)])],
            GREY,
            0
        ),
        PrimaryDirect::NothingOpaqueCovers
    );
    let small = (rect(0, 0, 150, 100), vec![rect(0, 0, 150, 100)]);
    assert_eq!(
        decide(vec![small], GREY, 0),
        PrimaryDirect::NothingOpaqueCovers
    );
    assert_eq!(
        decide(Vec::new(), GREY, 0),
        PrimaryDirect::NothingOpaqueCovers
    );
}

#[test]
fn over_a_wallpaper_an_alpha_window_is_never_what_smithay_tries() {
    // The review finding: an opaque wallpaper under an alpha window ends
    // the walk; it passes the guard, but it is not the window.
    assert_eq!(
        decide(vec![full(Vec::new()), full(whole())], GREY, 0),
        PrimaryDirect::NotTheWindow
    );
    // A transparent wallpaper over grey: nothing passes the guard.
    assert_eq!(
        decide(vec![full(Vec::new()), full(Vec::new())], GREY, 0),
        PrimaryDirect::NothingOpaqueCovers
    );
    // Over black, the guard passes for any last element -- which is the
    // wallpaper, not the window.
    for wallpaper in [full(whole()), full(Vec::new())] {
        assert_eq!(
            decide(vec![full(Vec::new()), wallpaper], BLACK, 0),
            PrimaryDirect::NotTheWindow
        );
    }
}

#[test]
fn over_black_an_alpha_window_alone_is_eligible() {
    // Smithay's other arm: over black the bottom element is tried whatever
    // its alpha, and here that is the window.
    assert_eq!(
        decide(vec![full(Vec::new())], BLACK, 0),
        PrimaryDirect::Eligible
    );
    let transparent = Color32F::new(0.3, 0.2, 0.1, 0.0);
    assert_eq!(
        decide(vec![full(Vec::new())], transparent, 0),
        PrimaryDirect::Eligible
    );
    let small = (rect(0, 0, 10, 10), Vec::new());
    assert_eq!(decide(vec![small], BLACK, 0), PrimaryDirect::Eligible);
}

#[test]
fn an_opaque_surface_above_the_window_is_not_the_window() {
    // Something above the covering window that is itself opaque over the
    // output ends the walk before the window is reached.
    assert_eq!(
        decide(vec![full(whole()), full(whole())], GREY, 1),
        PrimaryDirect::NotTheWindow
    );
}

#[test]
fn the_scratch_is_reused_across_frames() {
    // The per-frame allocation review asked about: after one frame has
    // grown the lists, the same frame again allocates nothing.
    let mut scratch = JudgeScratch::default();
    let many: Vec<Rectangle<i32, Physical>> =
        (0..20).map(|i| rect(i * 10, 0, 9, OUTPUT.1)).collect();
    let frame = || vec![full(many.clone()), full(Vec::new())];
    let _ = smithay_walk(frame(), OUTPUT, &mut scratch);
    let (opaque, work) = (scratch.opaque.capacity(), scratch.work.capacity());
    for _ in 0..10 {
        let _ = smithay_walk(frame(), OUTPUT, &mut scratch);
        assert_eq!(scratch.opaque.capacity(), opaque);
        assert_eq!(scratch.work.capacity(), work);
    }
}
