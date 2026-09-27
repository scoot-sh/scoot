use super::{Orientation, Walk};
use crate::image::fit::Rect;

/// A 3×2 stored image, pixels numbered in stored order:
///
/// ```text
/// 0 1 2
/// 3 4 5
/// ```
const W: u32 = 3;
const H: u32 = 2;

/// What each orientation displays, row by row, from the EXIF
/// specification's pictures of the letter F.
fn expected(value: u16) -> Vec<Vec<u32>> {
    match value {
        1 => vec![vec![0, 1, 2], vec![3, 4, 5]],
        2 => vec![vec![2, 1, 0], vec![5, 4, 3]],
        3 => vec![vec![5, 4, 3], vec![2, 1, 0]],
        4 => vec![vec![3, 4, 5], vec![0, 1, 2]],
        5 => vec![vec![0, 3], vec![1, 4], vec![2, 5]],
        6 => vec![vec![3, 0], vec![4, 1], vec![5, 2]],
        7 => vec![vec![5, 2], vec![4, 1], vec![3, 0]],
        8 => vec![vec![2, 5], vec![1, 4], vec![0, 3]],
        _ => unreachable!(),
    }
}

#[test]
fn every_orientation_maps_as_the_specification_says() {
    for value in 1..=8 {
        let o = Orientation::from_exif(value);
        assert_eq!(o.value(), value as u8);
        let (dw, dh) = o.displayed(W, H);
        let rows: Vec<Vec<u32>> = (0..dh)
            .map(|y| {
                (0..dw)
                    .map(|x| {
                        let (sx, sy) = o.to_stored(x, y, W, H);
                        sy * W + sx
                    })
                    .collect()
            })
            .collect();
        assert_eq!(rows, expected(value), "orientation {value}");
    }
}

#[test]
fn the_walk_agrees_with_the_mapping() {
    for value in 1..=8 {
        let o = Orientation::from_exif(value);
        let Walk { start, dx, dy } = o.walk(W, H);
        let (dw, dh) = o.displayed(W, H);
        for y in 0..dh {
            for x in 0..dw {
                let walked = start as isize + x as isize * dx + y as isize * dy;
                let (sx, sy) = o.to_stored(x, y, W, H);
                assert_eq!(walked, (sy * W + sx) as isize, "{value} at ({x},{y})");
            }
        }
    }
}

#[test]
fn a_displayed_rectangle_maps_to_the_stored_one_holding_the_same_pixels() {
    // A 5×4 image, every sub-rectangle, every orientation.
    let (w, h) = (5, 4);
    for value in 1..=8 {
        let o = Orientation::from_exif(value);
        let (dw, dh) = o.displayed(w, h);
        for x in 0..dw {
            for y in 0..dh {
                for rw in 1..=dw - x {
                    for rh in 1..=dh - y {
                        let rect = Rect {
                            x,
                            y,
                            width: rw,
                            height: rh,
                        };
                        let stored = o.rect_to_stored(rect, w, h);
                        assert_eq!(
                            (stored.width, stored.height),
                            o.stored(rw, rh),
                            "{value} {rect:?}"
                        );
                        assert!(stored.x + stored.width <= w && stored.y + stored.height <= h);
                        let mut from_display: Vec<(u32, u32)> = (y..y + rh)
                            .flat_map(|dy| (x..x + rw).map(move |dx| o.to_stored(dx, dy, w, h)))
                            .collect();
                        let mut in_stored: Vec<(u32, u32)> = (stored.y..stored.y + stored.height)
                            .flat_map(|sy| {
                                (stored.x..stored.x + stored.width).map(move |sx| (sx, sy))
                            })
                            .collect();
                        from_display.sort_unstable();
                        in_stored.sort_unstable();
                        assert_eq!(from_display, in_stored, "{value} {rect:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn out_of_range_values_are_shown_as_stored() {
    for value in [0, 9, 255, 256, 0x0106, u16::MAX] {
        assert_eq!(
            Orientation::from_exif(value),
            Orientation::NORMAL,
            "{value}"
        );
    }
    assert_eq!(Orientation::default(), Orientation::NORMAL);
}

#[test]
fn nothing_panics_out_of_range() {
    for value in 1..=8 {
        let o = Orientation::from_exif(value);
        // Coordinates past the image, and an empty image: saturate.
        let _ = o.to_stored(u32::MAX, u32::MAX, 3, 2);
        let _ = o.to_stored(5, 5, 0, 0);
        let _ = o.walk(0, 0);
        let _ = o.walk(u32::MAX, u32::MAX);
    }
}
