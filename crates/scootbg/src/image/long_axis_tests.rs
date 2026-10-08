//! Images with one very long side, through the real decode and render
//! entry points: the scaler's `f32` weights lose integer precision past
//! 2^24 pixels on an axis and index past their table (found in review of
//! PR #317: a 20,000,000×1 grey PNG, `--mode fit`, aborted the daemon), and
//! its weight tables grow with the axis. `scale::MAX_SCALED_SIDE` refuses
//! such an axis before the scaler sees it; these hold that for every mode
//! and filter, around the bound, and for tall images as well as wide.

use std::io::Cursor;

use super::decode::{Decoded, decode};
use super::fuzz::whole_path;
use super::render::{Look, RenderError, Source, render};
use super::scale::{MAX_SCALED_SIDE, ScaleError};
use super::{Filter, Mode, samples};
use crate::color::Color;

const FILL: Color = Color { r: 0, g: 0, b: 0 };

/// A flat grey PNG, `width` × `height`, made here: a 17-million-pixel row
/// compresses to about 17 KB, so nothing large is committed.
fn grey_png(width: u32, height: u32) -> Vec<u8> {
    let data = vec![0x80; width as usize * height as usize];
    samples::png(
        width,
        height,
        png::ColorType::Grayscale,
        png::BitDepth::Eight,
        &data,
        None,
    )
}

fn decoded(width: u32, height: u32) -> Decoded {
    decode(&mut Cursor::new(grey_png(width, height)), FILL).unwrap()
}

/// A tall image made straight into its decoded form: a PNG 17 million
/// rows tall is 17 million row filters, which a debug build decodes too
/// slowly for a unit test, and the decoder is not what is under test here.
fn decoded_tall(height: u32) -> Decoded {
    Decoded {
        rgb: vec![0x80; height as usize * 3],
        width: 1,
        height,
        orientation: super::orientation::Orientation::NORMAL,
    }
}

fn look(mode: Mode, filter: Filter) -> Look {
    Look {
        mode,
        fill: FILL,
        filter,
    }
}

/// Whether `result` is the long-axis refusal.
fn refused(result: &Result<scootbg_mem::ShmBuffer, RenderError>) -> bool {
    matches!(result, Err(RenderError::Scale(ScaleError::TooLong { .. })))
}

/// The review's case (decoded from a PNG) and its tall twin, past 2^24: `fit` and `stretch`
/// (which scale the long axis) are refused cleanly with every filter;
/// `fill` (which crops it first), `center` and `tile` (which do not scale)
/// draw.
#[test]
fn a_side_past_the_precision_cliff_is_refused_not_a_panic() {
    for (width, height) in [(17_000_000, 1), (1, 17_000_000)] {
        let image = if height == 1 {
            decoded(width, height)
        } else {
            decoded_tall(height)
        };
        for mode in Mode::ALL {
            // The filter matters only where the long side is scaled.
            let filters: &[Filter] = match mode {
                Mode::Fit | Mode::Stretch => &Filter::ALL,
                Mode::Fill | Mode::Center | Mode::Tile => &[Filter::Lanczos3],
            };
            for &filter in filters {
                let drawn = render(Source::Borrowed(&image), look(mode, filter), (1920, 1080));
                match mode {
                    Mode::Fit | Mode::Stretch => assert!(
                        refused(&drawn),
                        "{width}x{height} {mode:?} {filter:?}: {drawn:?}"
                    ),
                    Mode::Fill | Mode::Center | Mode::Tile => assert!(
                        drawn.is_ok(),
                        "{width}x{height} {mode:?} {filter:?}: {drawn:?}"
                    ),
                }
            }
        }
    }
}

/// The error says what to do about it.
#[test]
fn the_refusal_names_the_side_and_the_modes_that_work() {
    let image = decoded(17_000_000, 1);
    let Err(error) = render(
        Source::Owned(image),
        look(Mode::Fit, Filter::Lanczos3),
        (1920, 1080),
    ) else {
        panic!("drawn");
    };
    let text = error.to_string();
    assert!(text.contains("17000000"), "{text}");
    assert!(text.contains(&MAX_SCALED_SIDE.to_string()), "{text}");
    assert!(text.contains("fill"), "{text}");
}

/// Around the bound: a side of exactly `MAX_SCALED_SIDE` scales, one more
/// is refused, whether the long side is the source's or the target's.
#[test]
fn the_bound_is_exact_on_either_side_of_the_scaler() {
    for filter in Filter::ALL {
        for (from, to, allowed) in [
            ((MAX_SCALED_SIDE, 1), (1920, 1), true),
            ((MAX_SCALED_SIDE + 1, 1), (1920, 1), false),
            ((1, MAX_SCALED_SIDE), (1, 1080), true),
            ((1, MAX_SCALED_SIDE + 1), (1, 1080), false),
            ((4, 1), (MAX_SCALED_SIDE, 1), true),
            ((4, 1), (MAX_SCALED_SIDE + 1, 1), false),
            ((1, 4), (1, MAX_SCALED_SIDE + 1), false),
        ] {
            let source = vec![0x40; from.0 as usize * from.1 as usize * 3];
            let scaled = super::scale::scale(&source, from, to, filter);
            if allowed {
                assert!(scaled.is_ok(), "{from:?} -> {to:?} {filter:?}: {scaled:?}");
            } else {
                assert!(
                    matches!(scaled, Err(ScaleError::TooLong { .. })),
                    "{from:?} -> {to:?} {filter:?}: {scaled:?}"
                );
            }
        }
    }
}

/// A sweep of long sides through decode and render, from under the bound
/// to past 2^24, `fit` with the sharpest filter (the one that panicked):
/// under the bound it draws, past it it is refused, never a panic.
#[test]
fn a_sweep_of_long_sides_draws_or_refuses() {
    for width in [
        MAX_SCALED_SIDE / 2,
        MAX_SCALED_SIDE,
        MAX_SCALED_SIDE + 1,
        1 << 20,
        (1 << 24) - 1,
        1 << 24,
        (1 << 24) + 1,
        20_000_000,
    ] {
        let image = decoded(width, 1);
        let drawn = render(
            Source::Owned(image),
            look(Mode::Fit, Filter::Lanczos3),
            (2560, 1440),
        );
        if width <= MAX_SCALED_SIDE {
            assert!(drawn.is_ok(), "{width}: {drawn:?}");
        } else {
            assert!(refused(&drawn), "{width}: {drawn:?}");
        }
    }
}

/// The fuzz entry point, which runs the daemon's per-size loop, with the
/// review's input: the modes that scale the long side and the filters
/// that panicked, on a 1920×1080 and a 2560×1440 target.
#[test]
fn the_fuzz_entry_point_survives_the_reviews_input() {
    let file = grey_png(20_000_000, 1);
    // `Mode::ALL` indices 1 and 2 are `fit` and `stretch`; `Filter::ALL`
    // 0 and 1 are Lanczos3 and Catmull-Rom.
    for mode in [1_u8, 2] {
        for filter in [0_u8, 1] {
            let mut data = vec![mode, filter, 0, 0, 0, 0, 1];
            for side in [1920_u16, 1080, 2560, 1440, 0, 0] {
                data.extend_from_slice(&side.to_le_bytes());
            }
            data.extend_from_slice(&file);
            whole_path(&data);
        }
    }
}

/// The refusal's hint names only modes that can show the image: when the
/// output itself is the long side, `fill` scales to it too, so only the
/// modes that scale nothing are named.
#[test]
fn the_hint_names_only_modes_that_show_it() {
    let long_image = ScaleError::TooLong {
        from: (100_000, 10),
        to: (1920, 1080),
    }
    .to_string();
    assert!(long_image.contains("fill usually does"), "{long_image}");
    let long_output = ScaleError::TooLong {
        from: (10, 10),
        to: (70_000, 1),
    }
    .to_string();
    assert!(
        long_output.contains("center or tile shows it)"),
        "{long_output}"
    );
    assert!(!long_output.contains("fill"), "{long_output}");
}
