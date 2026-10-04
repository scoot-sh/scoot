//! The popup glue's pure parts: what is arithmetic, not protocol.

use super::events::device;
use super::resized;
use crate::density::Scale;
use crate::popup::Layout;

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
    assert!(!resized(&sized(300, 200), dims));
    // Either axis moving reopens: a menu grows from its `...` line,
    // and a scan that drops rows shrinks the list.
    for changed in [sized(300, 260), sized(240, 200), sized(0, 0)] {
        assert!(resized(&changed, dims));
    }
    // Once reopened at the new size, the same content does not reopen
    // again: one reopen per size change, never a loop.
    let grown = sized(300, 260);
    assert!(resized(&grown, dims));
    assert!(!resized(&grown, (grown.width, grown.height)));
}
