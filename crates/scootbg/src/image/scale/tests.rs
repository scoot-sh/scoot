use super::{ScaleError, rgb_len, scale};
use crate::image::Filter;

fn flat(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    rgb.repeat((width * height) as usize)
}

#[test]
fn a_flat_image_stays_flat_at_every_size_and_filter() {
    let rgb = [192, 48, 32];
    for filter in Filter::ALL {
        for (from, to) in [
            ((64, 40), (160, 100)),
            ((160, 100), (64, 40)),
            ((5, 5), (1, 1)),
        ] {
            let out = scale(&flat(from.0, from.1, rgb), from, to, filter).unwrap();
            assert_eq!(out.len(), (to.0 * to.1 * 3) as usize);
            for pixel in out.chunks_exact(3) {
                for (got, want) in pixel.iter().zip(rgb) {
                    assert!(
                        got.abs_diff(want) <= 1,
                        "{filter:?} {from:?}->{to:?}: {pixel:?}"
                    );
                }
            }
        }
    }
}

/// The scaler is safe code: a size it cannot handle would be a panic, and
/// a panic aborts the daemon. Every small shape and the extreme ones, every
/// filter, one axis or both, up and down: no panic, the right length.
#[test]
fn no_shape_panics_the_scaler() {
    let small: Vec<(u32, u32)> = (1..=5).flat_map(|w| (1..=5).map(move |h| (w, h))).collect();
    for filter in Filter::ALL {
        for &from in &small {
            let source = flat(from.0, from.1, [10, 200, 30]);
            for &to in &small {
                if from == to {
                    continue;
                }
                let out = scale(&source, from, to, filter).unwrap();
                assert_eq!(out.len(), (to.0 * to.1 * 3) as usize);
            }
        }
        for (from, to) in [
            ((1, 1), (3840, 2)),
            ((1, 1), (2, 2160)),
            ((20000, 1), (1, 1)),
            ((1, 20000), (7, 3)),
            ((20000, 2), (3, 40)),
            ((2, 3), (480, 270)),
            ((3000, 1), (1, 300)),
        ] {
            let out = scale(&flat(from.0, from.1, [1, 2, 3]), from, to, filter).unwrap();
            assert_eq!(
                out.len(),
                (to.0 * to.1 * 3) as usize,
                "{filter:?} {from:?}->{to:?}"
            );
        }
    }
}

#[test]
fn bad_sizes_are_errors_not_panics() {
    let source = flat(4, 4, [0, 0, 0]);
    assert_eq!(
        scale(&source, (0, 4), (2, 2), Filter::Lanczos3),
        Err(ScaleError::Empty)
    );
    assert_eq!(
        scale(&source, (4, 4), (2, 0), Filter::Lanczos3),
        Err(ScaleError::Empty)
    );
    // The slice is not the size it is said to be.
    assert_eq!(
        scale(&source, (4, 5), (2, 2), Filter::Lanczos3),
        Err(ScaleError::Size)
    );
    assert_eq!(
        scale(&source[..47], (4, 4), (2, 2), Filter::Lanczos3),
        Err(ScaleError::Size)
    );
    // Sizes whose byte count overflows.
    assert_eq!(
        scale(&source, (4, 4), (u32::MAX, u32::MAX), Filter::Nearest),
        Err(ScaleError::Size)
    );
    assert_eq!(
        scale(&source, (u32::MAX, u32::MAX), (2, 2), Filter::Nearest),
        Err(ScaleError::Size)
    );
    // The same size: the caller packs the source as it is.
    assert_eq!(
        scale(&source, (4, 4), (4, 4), Filter::Nearest),
        Err(ScaleError::Size)
    );
}

#[test]
fn byte_lengths_are_checked() {
    assert_eq!(rgb_len(3840, 2160), Some(3840 * 2160 * 3));
    assert_eq!(rgb_len(0, 5), Some(0));
    assert_eq!(rgb_len(u32::MAX, u32::MAX), None);
}

#[test]
fn nearest_keeps_hard_edges() {
    // Left half red, right half blue, doubled: still two exact halves.
    let mut source = Vec::new();
    for _ in 0..2 {
        source.extend_from_slice(&[255, 0, 0, 255, 0, 0, 0, 0, 255, 0, 0, 255]);
    }
    let out = scale(&source, (4, 2), (8, 4), Filter::Nearest).unwrap();
    for row in out.chunks_exact(8 * 3) {
        assert_eq!(&row[..12], [255, 0, 0].repeat(4).as_slice());
        assert_eq!(&row[12..], [0, 0, 255].repeat(4).as_slice());
    }
}
