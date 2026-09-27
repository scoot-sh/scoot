use super::{PackError, Stored, Target};
use crate::color::Color;
use crate::image::fit::Rect;
use crate::image::orientation::Orientation;

/// A stored image whose pixel `i` is `[i, i + 1, i + 2]` (mod 256), so
/// every pixel is distinct for small images.
fn numbered(width: u32, height: u32) -> Vec<u8> {
    (0..width * height)
        .flat_map(|i| [i as u8, (i + 1) as u8, (i + 2) as u8])
        .collect()
}

fn rgb_at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * width + x) * 4) as usize;
    let p = &pixels[i..i + 4];
    assert_eq!(p[3], 0xff, "the unused byte is opaque");
    [p[2], p[1], p[0]]
}

#[test]
fn every_orientation_draws_the_displayed_image() {
    // Wide enough for more than one 32-pixel block each way.
    let (w, h) = (70, 37);
    let rgb = numbered(w, h);
    for value in 1..=8 {
        let o = Orientation::from_exif(value);
        let (dw, dh) = o.displayed(w, h);
        let mut pixels = vec![0; (dw * dh * 4) as usize];
        let mut target = Target::new(&mut pixels, dw, dh).unwrap();
        let image = Stored {
            rgb: &rgb,
            width: w,
            height: h,
            orientation: o,
        };
        target.draw(image, Rect::whole(dw, dh), (0, 0)).unwrap();
        for y in 0..dh {
            for x in 0..dw {
                let (sx, sy) = o.to_stored(x, y, w, h);
                let i = ((sy * w + sx) * 3) as usize;
                assert_eq!(
                    rgb_at(&pixels, dw, x, y),
                    [rgb[i], rgb[i + 1], rgb[i + 2]],
                    "orientation {value} at ({x},{y})"
                );
            }
        }
    }
}

#[test]
fn a_region_lands_where_asked_and_nothing_else_is_touched() {
    let (w, h) = (9, 6);
    let rgb = numbered(w, h);
    for value in 1..=8 {
        let o = Orientation::from_exif(value);
        let (dw, dh) = o.displayed(w, h);
        let region = Rect {
            x: 1,
            y: 2,
            width: dw - 3,
            height: dh - 2,
        };
        let (tw, th) = (20, 20);
        let mut pixels = vec![7; (tw * th * 4) as usize];
        let mut target = Target::new(&mut pixels, tw, th).unwrap();
        let image = Stored {
            rgb: &rgb,
            width: w,
            height: h,
            orientation: o,
        };
        target.draw(image, region, (5, 4)).unwrap();
        for y in 0..th {
            for x in 0..tw {
                let inside =
                    (5..5 + region.width).contains(&x) && (4..4 + region.height).contains(&y);
                let i = ((y * tw + x) * 4) as usize;
                if inside {
                    let (sx, sy) = o.to_stored(region.x + x - 5, region.y + y - 4, w, h);
                    let s = ((sy * w + sx) * 3) as usize;
                    assert_eq!(rgb_at(&pixels, tw, x, y), [rgb[s], rgb[s + 1], rgb[s + 2]]);
                } else {
                    assert_eq!(pixels[i..i + 4], [7, 7, 7, 7], "touched ({x},{y})");
                }
            }
        }
    }
}

#[test]
fn out_of_range_draws_are_errors() {
    let rgb = numbered(4, 3);
    let image = |o| Stored {
        rgb: &rgb,
        width: 4,
        height: 3,
        orientation: Orientation::from_exif(o),
    };
    let mut pixels = vec![0; 10 * 10 * 4];
    let mut target = Target::new(&mut pixels, 10, 10).unwrap();
    // A region past the displayed image (3×4 when rotated).
    assert_eq!(
        target.draw(image(6), Rect::whole(4, 3), (0, 0)),
        Err(PackError)
    );
    assert_eq!(
        target.draw(image(1), Rect::whole(0, 3), (0, 0)),
        Err(PackError)
    );
    // Placed past the buffer.
    assert_eq!(
        target.draw(image(1), Rect::whole(4, 3), (7, 0)),
        Err(PackError)
    );
    assert_eq!(
        target.draw(image(1), Rect::whole(4, 3), (u32::MAX, 0)),
        Err(PackError)
    );
    // A slice shorter than the image claims.
    let short = Stored {
        rgb: &rgb[..35],
        ..image(1)
    };
    assert_eq!(
        target.draw(short, Rect::whole(4, 3), (0, 0)),
        Err(PackError)
    );
    // Sizes whose byte count overflows.
    let huge = Stored {
        width: u32::MAX,
        height: u32::MAX,
        ..image(1)
    };
    assert_eq!(target.draw(huge, Rect::whole(1, 1), (0, 0)), Err(PackError));
    // A buffer smaller than it says.
    let mut small = vec![0; 15];
    assert!(Target::new(&mut small, 2, 2).is_none());
}

#[test]
fn fill_around_paints_only_the_bars() {
    let (tw, th) = (12, 9);
    let mut pixels = vec![0; (tw * th * 4) as usize];
    let mut target = Target::new(&mut pixels, tw, th).unwrap();
    let placed = Rect {
        x: 3,
        y: 2,
        width: 5,
        height: 4,
    };
    target.fill_around(placed, Color { r: 1, g: 2, b: 3 });
    for y in 0..th {
        for x in 0..tw {
            let inside = (3..8).contains(&x) && (2..6).contains(&y);
            let i = ((y * tw + x) * 4) as usize;
            let want = if inside {
                [0, 0, 0, 0]
            } else {
                [3, 2, 1, 0xff]
            };
            assert_eq!(pixels[i..i + 4], want, "({x},{y})");
        }
    }
    // Placed over everything, or at an edge: nothing past the buffer.
    let mut target = Target::new(&mut pixels, tw, th).unwrap();
    target.fill_around(Rect::whole(tw, th), Color { r: 9, g: 9, b: 9 });
    target.fill_around(
        Rect {
            x: tw,
            y: th,
            width: u32::MAX,
            height: u32::MAX,
        },
        Color { r: 4, g: 4, b: 4 },
    );
    assert!(pixels.chunks_exact(4).all(|p| p == [4, 4, 4, 0xff]));
}

#[test]
fn repeat_tiles_from_the_top_left() {
    for (tw, th, tile) in [
        (13, 7, (3, 2)),
        (8, 8, (8, 8)),
        (5, 9, (1, 1)),
        (40, 3, (7, 3)),
        (6, 6, (9, 9)),
    ] {
        let mut pixels = vec![0; (tw * th * 4) as usize];
        let (cw, ch) = (tile.0.min(tw), tile.1.min(th));
        let rgb = numbered(cw, ch);
        let mut target = Target::new(&mut pixels, tw, th).unwrap();
        let image = Stored {
            rgb: &rgb,
            width: cw,
            height: ch,
            orientation: Orientation::NORMAL,
        };
        target.draw(image, Rect::whole(cw, ch), (0, 0)).unwrap();
        target.repeat(tile);
        for y in 0..th {
            for x in 0..tw {
                assert_eq!(
                    rgb_at(&pixels, tw, x, y),
                    rgb_at(&pixels, tw, x % cw, y % ch),
                    "{tw}x{th} tile {tile:?} at ({x},{y})"
                );
            }
        }
    }
}
