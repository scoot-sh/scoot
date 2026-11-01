//! The popup glue's pure parts: what is arithmetic, not protocol.

use super::events::device;
use super::{fits, popup_rects, resized, rest_origin};
use crate::density::Scale;
use crate::popup::Layout;
use scootui::paint::Pad;

fn no_pad() -> Pad {
    Pad {
        top: 0,
        right: 0,
        bottom: 0,
        left: 0,
    }
}

#[test]
fn a_pointer_coordinate_is_device_pixels_at_the_scale_and_may_be_negative() {
    assert_eq!(device(10.0, Scale::Integer(1)), 10);
    assert_eq!(device(10.0, Scale::Integer(2)), 20);
    // 1.5x: 7 logical is 10.5 device, truncated toward zero.
    assert_eq!(device(7.0, Scale::Fractional(180)), 10);
    // A drag past the popup's left or top edge is negative, not clamped to
    // the edge (the slider clamps to its track itself).
    assert_eq!(device(-4.0, Scale::Integer(2)), -8);
}

#[test]
fn a_hostile_coordinate_is_bounded_and_a_non_finite_one_is_zero() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(device(bad, Scale::Integer(1)), 0);
    }
    assert_eq!(device(1.0e300, Scale::Integer(8)), 1_000_000_000);
    assert_eq!(device(-1.0e300, Scale::Fractional(480)), -1_000_000_000);
}

#[test]
fn a_refill_at_another_size_reopens_and_at_the_same_size_does_not() {
    fn sized(width: u32, height: u32) -> Layout {
        let mut layout = Layout::default();
        layout.width = width;
        layout.height = height;
        layout
    }
    let dims = (300, 200);
    // What the open earned: the reopen reuses this surface's size as
    // the "still the same" baseline, so the decision is purely the
    // computed size against it.
    assert!(!resized(&sized(300, 200), no_pad(), dims));
    // Either axis moving reopens: a menu grows from its `...` line,
    // and a scan that drops rows shrinks the list.
    for changed in [sized(300, 260), sized(240, 200), sized(0, 0)] {
        assert!(resized(&changed, no_pad(), dims));
    }
    // Once reopened at the new size, the same content does not reopen
    // again: one reopen per size change, never a loop.
    let grown = sized(300, 260);
    assert!(resized(&grown, no_pad(), dims));
    assert!(!resized(&grown, no_pad(), (grown.width, grown.height)));
}

#[test]
fn a_restyled_popup_is_remade_only_when_it_no_longer_fits() {
    fn sized(width: u32, height: u32) -> Layout {
        let mut layout = Layout::default();
        layout.width = width;
        layout.height = height;
        layout
    }
    // Scale 1: the buffer is the size itself. Smaller keeps the surface
    // (no flicker); bigger is made again.
    assert!(fits(
        Scale::Integer(1),
        &sized(300, 200),
        no_pad(),
        (300, 200)
    ));
    assert!(fits(
        Scale::Integer(1),
        &sized(240, 200),
        no_pad(),
        (300, 200)
    ));
    assert!(!fits(
        Scale::Integer(1),
        &sized(300, 201),
        no_pad(),
        (300, 200)
    ));
    assert!(!fits(
        Scale::Integer(1),
        &sized(301, 200),
        no_pad(),
        (300, 200)
    ));
    // Scale 2 with an odd device width: the buffer rounds up (101 device
    // is 51 logical, 102 buffered), and the same content still fits it.
    // The old device-size comparison remade here, at every reload.
    assert!(fits(
        Scale::Integer(2),
        &sized(101, 50),
        no_pad(),
        (102, 100)
    ));
    assert!(fits(
        Scale::Integer(2),
        &sized(100, 50),
        no_pad(),
        (102, 100)
    ));
    assert!(!fits(
        Scale::Integer(2),
        &sized(103, 50),
        no_pad(),
        (102, 100)
    ));
    assert!(!fits(
        Scale::Integer(2),
        &sized(101, 101),
        no_pad(),
        (102, 100)
    ));
    // Fractional 1.5: 101 device is 68 logical, 102 buffered.
    assert!(fits(
        Scale::Fractional(180),
        &sized(101, 50),
        no_pad(),
        (102, 51)
    ));
    assert!(!fits(
        Scale::Fractional(180),
        &sized(103, 50),
        no_pad(),
        (102, 51)
    ));
}

/// The input region follows the popup every frame in motion (where it is
/// drawn, not where it rests): different origins give different rects, so
/// clicks during the entrance hit where the popup is, and the same origin
/// gives the same rects, so at rest every draw but the first after a change
/// sends nothing. The rects key on everything that changes them (origin,
/// size, radius): a runtime look switch that changes any of them rebuilds.
#[test]
fn the_input_region_follows_the_popup_and_keys_on_everything() {
    let mut rest = Vec::new();
    let mut moved = Vec::new();
    let mut same = Vec::new();
    // Square: one rect at the origin.
    popup_rects(100, 50, 0, (28, 24), &mut rest);
    popup_rects(100, 50, 0, (28, 18), &mut moved);
    assert_ne!(rest, moved, "a slide moves the region");
    popup_rects(100, 50, 0, (28, 24), &mut same);
    assert_eq!(rest, same, "the same origin sends nothing anew");
    // Size and radius change the rects too: the cache key is the content.
    let mut resized = Vec::new();
    popup_rects(120, 50, 0, (28, 24), &mut resized);
    assert_ne!(rest, resized, "a new size rebuilds the region");
    let mut rounded = Vec::new();
    let mut square = Vec::new();
    popup_rects(100, 50, 6, (28, 24), &mut rounded);
    popup_rects(100, 50, 0, (28, 24), &mut square);
    assert_ne!(rounded, square, "a new radius rebuilds the region");
}

/// The rest origin keeps the motion room without a shadow (B1): a
/// shadowless moving popup rests at `(pad.left, pad.top)`, not `(0, 0)` —
/// `(0, 0)` pins every upward slide frame to the surface edge via the clamp
/// in `draw_open`, so the entrance never moves. Tooltips keep `(0, 0)`.
#[test]
fn a_shadowless_moving_popup_rests_at_the_pad_not_the_origin() {
    // Shadowless with motion room (the default shadow, an active timing):
    // the 16 px room sits above the popup.
    assert_eq!(rest_origin(true, 16, 16), (16, 16));
    // Shadowed: the shadow's pad plus the same room.
    assert_eq!(rest_origin(true, 28, 24), (28, 24));
    // Still popups without a shadow keep the surface they always had.
    assert_eq!(rest_origin(true, 0, 0), (0, 0));
    // Tooltips never move: the origin whatever the pad.
    assert_eq!(rest_origin(false, 0, 0), (0, 0));
    assert_eq!(rest_origin(false, 16, 16), (0, 0));
}

/// Warm animated frames allocate nothing: rebuilding the pooled region
/// rects at a new origin every frame (a slide) builds nothing on the heap
/// once the lists are warm. Shadow tables are prepared once on open (the
/// bar's `warm_frames` test pins the table cache); `none` builds nothing.
#[test]
fn warm_entrance_frames_allocate_nothing_once_pooled() {
    use scootbg_mem::count_allocations;
    let mut scratch = Vec::new();
    let mut sent = Vec::new();
    // Warm up: size both pooled lists once, as the open does (one send,
    // one scratch).
    popup_rects(120, 60, 6, (28, 24), &mut sent);
    popup_rects(120, 60, 6, (28, 24), &mut scratch);
    // Ten animated frames (rest and 6 px above, alternating, as a slide
    // would) allocate nothing.
    let ((), allocations) = count_allocations(|| {
        for i in 0..10 {
            let oy = if i % 2 == 0 { 24 } else { 18 };
            popup_rects(120, 60, 6, (28, oy), &mut scratch);
            std::mem::swap(&mut scratch, &mut sent);
        }
    });
    assert_eq!(allocations, 0, "warm entrance frames allocated");
}
