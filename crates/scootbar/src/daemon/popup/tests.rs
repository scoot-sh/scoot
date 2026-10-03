//! The popup glue's pure parts: what is arithmetic, not protocol.

use super::events::device;
use crate::density::Scale;

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
