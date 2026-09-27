use super::{Buffer, DENOMINATOR, Preferred, Scale, scaled_length};
use crate::outputs::Size;

fn size(width: u32, height: u32) -> Size {
    Size { width, height }
}

/// Modes people have, landscape; each is also checked rotated.
const MODES: [(u32, u32); 16] = [
    (1024, 768),
    (1280, 720),
    (1280, 800),
    (1366, 768),
    (1600, 900),
    (1600, 1000),
    (1920, 1080),
    (1920, 1200),
    (2256, 1504),
    (2560, 1440),
    (2560, 1600),
    (2880, 1800),
    (3000, 2000),
    (3440, 1440),
    (3840, 2160),
    (7680, 4320),
];

/// Every scale step `wp_fractional_scale_v1` can say across what scoot
/// allows (0.5 to 4), in 120ths.
fn steps() -> impl Iterator<Item = u32> {
    60..=480
}

/// What the compositor draws a surface of `logical` pixels into at
/// `v120`/120, in device pixels: `round(logical × scale)`, halves away from
/// zero. Smithay (`WaylandSurfaceRenderElement::size`) and wlroots
/// (`scale_length`) both do this at the origin. Worked out in `f64`
/// independently of the integer arithmetic under test: `logical × v120` is
/// exact, and so is a quotient ending in .5.
fn compositor_dst(logical: u32, v120: u32) -> u32 {
    let exact = f64::from(logical) * f64::from(v120) / f64::from(DENOMINATOR);
    exact.round() as u32
}

#[test]
fn the_protocol_rounds_halves_away_from_zero() {
    // The protocol's own example: 100×50 at 1.5 is 150×75.
    assert_eq!(scaled_length(100, 180), Some(150));
    assert_eq!(scaled_length(50, 180), Some(75));
    // The ticket's case: 2560 px at 1.5 is 1707 logical (rounded up, as
    // scoot does), and 1707 × 1.5 = 2560.5 rounds to 2561.
    assert_eq!(scaled_length(1707, 180), Some(2561));
    // wlroots truncates the logical size to 1706: 2559.
    assert_eq!(scaled_length(1706, 180), Some(2559));
    // 1600×1000 at 1.5: scoot's 1067×667 is drawn 1601×1001.
    assert_eq!(scaled_length(1067, 180), Some(1601));
    assert_eq!(scaled_length(667, 180), Some(1001));
    // Exact at integer scales.
    assert_eq!(scaled_length(800, 240), Some(1600));
    assert_eq!(scaled_length(1600, 120), Some(1600));
    // Just below and above a half.
    assert_eq!(scaled_length(1, 179), Some(1)); // 1.49
    assert_eq!(scaled_length(3, 140), Some(4)); // 3.5
    assert_eq!(scaled_length(3, 139), Some(3)); // 3.475
}

#[test]
fn a_buffer_side_is_never_zero_and_never_overflows() {
    assert_eq!(scaled_length(1, 60), Some(1), "0.5 rounds up to 1");
    assert_eq!(scaled_length(1, 1), Some(1), "1/120 would be 0");
    assert_eq!(scaled_length(0, 180), Some(1), "no 0 side");
    assert_eq!(scaled_length(u32::MAX, 120), Some(u32::MAX));
    assert_eq!(scaled_length(u32::MAX, 121), None);
    assert_eq!(scaled_length(u32::MAX, u32::MAX), None);
    assert_eq!(Scale::Fractional(480).buffer(size(u32::MAX, 1)), None);
    assert_eq!(Scale::Integer(2).buffer(size(1, u32::MAX)), None);
}

/// For every mode, rotated or not, and every scale step, the buffer is
/// exactly the rectangle the compositor draws the surface into, whichever
/// way the compositor rounded the logical size (scoot up, wlroots down), so
/// it lands one buffer pixel to one device pixel. That rectangle is not
/// always the mode: rounding the logical size up covers the output and
/// overhangs it, rounding it down leaves a gap, each by less than the scale
/// plus the destination's half pixel (2 pixels for 1366 at 4, for one).
#[test]
fn every_scale_step_on_every_mode_is_one_to_one() {
    let mut checked = 0;
    for (w, h) in MODES {
        for (width, height) in [(w, h), (h, w)] {
            for v120 in steps() {
                let scale = f64::from(v120) / f64::from(DENOMINATOR);
                for (logical_w, logical_h, rounding) in [
                    // Smithay: ceil(mode / scale).
                    (
                        (f64::from(width) / scale).ceil() as u32,
                        (f64::from(height) / scale).ceil() as u32,
                        "up",
                    ),
                    // wlroots: truncated.
                    (
                        (f64::from(width) / scale) as u32,
                        (f64::from(height) / scale) as u32,
                        "down",
                    ),
                ] {
                    let buffer = Scale::Fractional(v120)
                        .buffer(size(logical_w, logical_h))
                        .unwrap();
                    let want = (
                        compositor_dst(logical_w, v120),
                        compositor_dst(logical_h, v120),
                    );
                    assert_eq!(
                        buffer.dims, want,
                        "{width}x{height} at {v120}/120, logical rounded {rounding}"
                    );
                    assert_eq!(buffer.scale, 1);
                    let within = |got: u32, mode: u32| {
                        let bound = f64::from(got.abs_diff(mode)) < scale + 0.5;
                        match rounding {
                            "up" => got >= mode && bound,
                            _ => got <= mode && bound,
                        }
                    };
                    assert!(
                        within(buffer.dims.0, width) && within(buffer.dims.1, height),
                        "{width}x{height} at {v120}/120 ({rounding}): {:?}",
                        buffer.dims
                    );
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, MODES.len() * 2 * 421 * 2);
}

/// Where the mode divides exactly, the buffer is the mode, however the
/// compositor rounds: 2880×1800 at 1.5, 3840×2160 at 1.25 and 2, and so on.
#[test]
fn an_exact_scale_gives_the_mode() {
    for ((width, height), v120) in [
        ((2880, 1800), 180),
        ((3840, 2160), 150),
        ((3840, 2160), 240),
        ((1920, 1080), 180),
        ((2560, 1600), 160),
        ((1600, 1000), 120),
    ] {
        let logical = Scale::Fractional(v120).logical(size(width, height));
        assert_eq!(
            Scale::Fractional(v120).buffer(logical).map(|b| b.dims),
            Some((width, height)),
            "{width}x{height} at {v120}/120"
        );
    }
}

#[test]
fn an_integer_scale_multiplies_and_sets_the_buffer_scale() {
    assert_eq!(
        Scale::Integer(2).buffer(size(800, 500)),
        Some(Buffer {
            dims: (1600, 1000),
            scale: 2
        })
    );
    // wl_output's 2 for a 1.5 output: larger than the output, as before.
    assert_eq!(
        Scale::Integer(2).buffer(size(1067, 667)).map(|b| b.dims),
        Some((2134, 1334))
    );
    assert_eq!(
        Scale::Integer(0).buffer(size(10, 10)).map(|b| b.scale),
        Some(1),
        "never 0"
    );
}

#[test]
fn a_buffer_fits_its_surface_only_at_its_own_scale() {
    let fits = |dims, scale, w, h| Buffer { dims, scale }.fits(size(w, h));
    assert!(fits((1600, 1000), 1, 1600, 1000));
    assert!(fits((1600, 1000), 2, 800, 500));
    assert!(!fits((1601, 1001), 1, 1067, 667), "fractional: a viewport");
    assert!(!fits((1, 1), 1, 1600, 1000), "a 1×1 color: a viewport");
    assert!(fits((1, 1), 1, 1, 1));
    assert!(!fits((u32::MAX, 1), 2, u32::MAX, 1), "no overflow");
}

#[test]
fn the_logical_size_follows_the_scale_known() {
    let mode = size(1600, 1000);
    assert_eq!(Scale::Integer(1).logical(mode), mode);
    assert_eq!(Scale::Integer(2).logical(mode), size(800, 500));
    // A fractional scale rounds up, as scoot does: the surface covers it.
    assert_eq!(Scale::Fractional(180).logical(mode), size(1067, 667));
    assert_eq!(Scale::Fractional(150).logical(mode), size(1280, 800));
    assert_eq!(Scale::Fractional(60).logical(mode), size(3200, 2000));
    // Never 0 on a side that has pixels; 0 stays 0.
    assert_eq!(Scale::Integer(4).logical(size(3, 0)), size(1, 0));
    assert_eq!(Scale::Fractional(480).logical(size(1, 1)), size(1, 1));
    assert_eq!(
        Scale::Fractional(1).logical(size(u32::MAX, 1)),
        size(u32::MAX, 120),
        "saturates"
    );
}

#[test]
fn the_best_scale_known_wins() {
    let mut preferred = Preferred::default();
    assert_eq!(preferred.scale(2), Scale::Integer(2), "wl_output's");
    assert_eq!(preferred.scale(0), Scale::Integer(1), "never 0");
    assert!(preferred.set_buffer_scale(3));
    assert_eq!(preferred.scale(2), Scale::Integer(3), "the surface's");
    assert!(preferred.set_fractional(180));
    assert_eq!(preferred.scale(2), Scale::Fractional(180), "the fraction");
    // A fraction below 1 rounds up to 1, as `wl_output` says.
    assert!(preferred.set_fractional(60));
    assert_eq!(preferred.scale(1), Scale::Fractional(60));
    assert!(preferred.set_fractional(180));
    assert_eq!(Scale::Fractional(180).value(), 1.5);
    assert_eq!(Scale::Integer(2).value(), 2.0);
    assert_eq!(Scale::Fractional(150).to_string(), "1.25");
    let json = |scale| serde_json::to_string(&scale).unwrap();
    assert_eq!(json(Scale::Integer(2)), "2");
    assert_eq!(json(Scale::Fractional(240)), "2", "a whole fraction");
    assert_eq!(json(Scale::Fractional(180)), "1.5");
    assert_eq!(json(Scale::Fractional(160)), "1.3333333333333333");
}

#[test]
fn a_repeated_or_broken_scale_changes_nothing() {
    let mut preferred = Preferred::default();
    assert!(!preferred.set_fractional(0), "0 is ignored");
    assert!(!preferred.set_buffer_scale(0));
    assert!(!preferred.set_buffer_scale(-2));
    assert_eq!(preferred, Preferred::default());
    assert!(preferred.set_fractional(180));
    assert!(!preferred.set_fractional(180), "the same again");
    assert!(preferred.set_fractional(150));
    assert!(preferred.set_buffer_scale(2));
    assert!(!preferred.set_buffer_scale(2));
    assert!(!preferred.set_fractional(0), "keeps the last good one");
    assert_eq!(preferred.fractional, Some(150));
}

/// sway changed the output to 1.5 while the surface was not on screen:
/// `wl_output` says 2, but the surface still has the 1.0 it was made with.
/// The larger wins until the compositor says otherwise: drawn for more
/// pixels than the output has (scaled down, sharp), never fewer (scaled
/// up, a blur).
#[test]
fn a_stale_smaller_scale_gives_way_to_the_output() {
    let mut preferred = Preferred::default();
    assert!(preferred.set_fractional(120));
    assert!(preferred.set_buffer_scale(1));
    assert_eq!(preferred.scale(2), Scale::Integer(2));
    // The compositor catches up.
    assert!(preferred.set_fractional(180));
    assert_eq!(preferred.scale(2), Scale::Fractional(180));
    // The other way round (1.5 lowered to 1, the fraction stale): the
    // fraction is the larger, and stays until corrected.
    assert_eq!(preferred.scale(1), Scale::Fractional(180));
    // A compositor that says 1 on `wl_output` but prefers 1.5 for the
    // surface is followed, not second-guessed.
    assert_eq!(preferred.scale(1), Scale::Fractional(180));
    // Integers alike: the larger of the surface's and the output's.
    let mut integer = Preferred::default();
    assert!(integer.set_buffer_scale(1));
    assert_eq!(integer.scale(3), Scale::Integer(3));
    assert!(integer.set_buffer_scale(2));
    assert_eq!(integer.scale(1), Scale::Integer(2));
}
