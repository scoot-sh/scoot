use super::{DENOMINATOR, Preferred, Scale, scaled_length};
use crate::outputs::Size;

fn size(width: u32, height: u32) -> Size {
    Size { width, height }
}

/// What a compositor draws a surface of `logical` pixels into at
/// `v120`/120: `round(logical × scale)`, halves away from zero, as Smithay
/// and wlroots both do. Worked out in `f64`, independently of the integer
/// arithmetic under test (`logical × v120` is exact there, and so is a
/// quotient ending in .5).
fn compositor_dst(logical: u32, v120: u32) -> u32 {
    let exact = f64::from(logical) * f64::from(v120) / f64::from(DENOMINATOR);
    exact.round() as u32
}

#[test]
fn the_protocol_rounds_halves_away_from_zero() {
    assert_eq!(scaled_length(100, 180), Some(150));
    assert_eq!(scaled_length(28, 180), Some(42));
    // 1707 × 1.5 = 2560.5 rounds up.
    assert_eq!(scaled_length(1707, 180), Some(2561));
    assert_eq!(scaled_length(1600, 120), Some(1600));
    // 28 × 1.25 = 35; 29 × 1.25 = 36.25.
    assert_eq!(scaled_length(28, 150), Some(35));
    assert_eq!(scaled_length(29, 150), Some(36));
}

#[test]
fn every_bar_height_at_every_scale_matches_the_compositor() {
    for v120 in 60..=480 {
        for height in 1..=200 {
            assert_eq!(
                scaled_length(height, v120),
                Some(compositor_dst(height, v120).max(1)),
                "{height} at {v120}/120"
            );
        }
    }
}

#[test]
fn a_side_is_never_zero_and_overflow_is_none() {
    assert_eq!(scaled_length(0, 180), Some(1));
    assert_eq!(scaled_length(1, 60), Some(1));
    assert_eq!(scaled_length(1, 0), Some(1));
    assert_eq!(scaled_length(u32::MAX, 240), None);
    assert_eq!(Scale::Integer(2).buffer(size(u32::MAX, 28)), None);
    assert_eq!(Scale::Integer(2).buffer(size(28, u32::MAX)), None);
    assert_eq!(Scale::Fractional(480).buffer(size(u32::MAX / 2, 1)), None);
}

#[test]
fn buffers_at_integer_and_fractional_scales() {
    assert_eq!(Scale::Integer(1).buffer(size(1920, 28)), Some((1920, 28)));
    assert_eq!(Scale::Integer(2).buffer(size(1920, 28)), Some((3840, 56)));
    // A zero factor cannot come from `Preferred`; treated as 1 all the same.
    assert_eq!(Scale::Integer(0).buffer(size(1920, 28)), Some((1920, 28)));
    assert_eq!(
        Scale::Fractional(180).buffer(size(1280, 28)),
        Some((1920, 42))
    );
    assert_eq!(
        Scale::Fractional(150).buffer(size(1536, 29)),
        Some((1920, 36))
    );
}

#[test]
fn the_integer_factor_of_a_fraction_rounds_up() {
    assert_eq!(Scale::Fractional(120).integer(), 1);
    assert_eq!(Scale::Fractional(150).integer(), 2);
    assert_eq!(Scale::Fractional(240).integer(), 2);
    assert_eq!(Scale::Fractional(0).integer(), 1);
    assert_eq!(Scale::Integer(3).integer(), 3);
    assert_eq!(Scale::Integer(0).integer(), 1);
}

#[test]
fn logical_sizes_round_as_the_module_says() {
    assert_eq!(
        Scale::Integer(2).logical(size(3840, 2160)),
        size(1920, 1080)
    );
    assert_eq!(Scale::Integer(2).logical(size(1, 3)), size(1, 1));
    assert_eq!(
        Scale::Fractional(180).logical(size(2560, 1600)),
        size(1707, 1067)
    );
    assert_eq!(Scale::Fractional(180).logical(size(0, 0)), size(0, 0));
    assert_eq!(Scale::Fractional(0).logical(size(u32::MAX, 1)).height, 120);
}

#[test]
fn the_best_scale_known_wins() {
    let mut preferred = Preferred::default();
    assert_eq!(preferred.scale(1), Scale::Integer(1));
    assert_eq!(preferred.scale(2), Scale::Integer(2));
    // A zero output scale cannot come from `Output`; treated as 1.
    assert_eq!(preferred.scale(0), Scale::Integer(1));
    preferred.set_buffer_scale(3);
    assert_eq!(preferred.scale(2), Scale::Integer(3));
    preferred.set_fractional(180);
    assert_eq!(preferred.scale(2), Scale::Fractional(180));
    // Stale: 1.5 rounds up to 2, below an output scale of 3.
    assert_eq!(preferred.scale(3), Scale::Integer(3));
}

#[test]
fn broken_scales_are_ignored() {
    let mut preferred = Preferred::default();
    preferred.set_fractional(0);
    preferred.set_buffer_scale(0);
    preferred.set_buffer_scale(-2);
    assert_eq!(preferred, Preferred::default());
    preferred.set_fractional(150);
    preferred.set_fractional(0);
    assert_eq!(preferred.fractional, Some(150));
}
