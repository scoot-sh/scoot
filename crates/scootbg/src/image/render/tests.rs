use super::{Look, RenderError, Source, crop_in_place, render};
use crate::color::Color;
use crate::image::decode::Decoded;
use crate::image::fit::Rect;
use crate::image::orientation::Orientation;
use crate::image::{Filter, Mode};

const FILL: Color = Color { r: 1, g: 2, b: 3 };

fn look(mode: Mode) -> Look {
    Look {
        mode,
        fill: FILL,
        filter: Filter::Lanczos3,
    }
}

/// A displayed image in four flat quadrants: red, green / blue, white.
fn quadrant(x: u32, y: u32, width: u32, height: u32) -> [u8; 3] {
    match (x < width / 2, y < height / 2) {
        (true, true) => [255, 0, 0],
        (false, true) => [0, 255, 0],
        (true, false) => [0, 0, 255],
        (false, false) => [255, 255, 255],
    }
}

/// The stored form of a displayed `width` × `height` image `pixel(x, y)`
/// under orientation `value`: what a camera would have written.
fn stored(value: u16, width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Decoded {
    let o = Orientation::from_exif(value);
    let (sw, sh) = o.stored(width, height);
    let mut rgb = vec![0; (sw * sh * 3) as usize];
    for y in 0..height {
        for x in 0..width {
            let (sx, sy) = o.to_stored(x, y, sw, sh);
            let i = ((sy * sw + sx) * 3) as usize;
            rgb[i..i + 3].copy_from_slice(&pixel(x, y));
        }
    }
    Decoded {
        rgb,
        width: sw,
        height: sh,
        orientation: o,
    }
}

fn pixels(buffer: &mut scootbg_mem::ShmBuffer) -> Vec<[u8; 3]> {
    buffer
        .pixels_mut()
        .chunks_exact(4)
        .map(|p| {
            assert_eq!(p[3], 0xff);
            [p[2], p[1], p[0]]
        })
        .collect()
}

fn close(a: [u8; 3], b: [u8; 3], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tolerance)
}

/// Smooth content: gradients, no hard edges.
fn smooth(x: u32, y: u32) -> [u8; 3] {
    [(x * 6) as u8, (y * 10) as u8, ((x + y) * 3) as u8]
}

#[test]
fn every_orientation_renders_what_is_displayed() {
    // A displayed 40×24 image, stored eight ways; each rendered in every
    // mode must match the upright one. Scaling runs in the stored
    // orientation, so for the 90° cases the scaler's two passes run the
    // other way round and round differently: on smooth content by a step
    // or two (hard edges, where the first pass clamps Lanczos ringing,
    // differ by more; that is the filter, not the orientation).
    let (w, h) = (40, 24);
    let upright = stored(1, w, h, smooth);
    for mode in Mode::ALL {
        for dims in [(64, 30), (30, 64), (20, 12), (40, 24)] {
            let mut want = render(Source::Borrowed(&upright), look(mode), dims).unwrap();
            let want = pixels(&mut want);
            for value in 2..=8 {
                let image = stored(value, w, h, smooth);
                let mut got = render(Source::Owned(image), look(mode), dims).unwrap();
                let got = pixels(&mut got);
                let worst = want
                    .iter()
                    .zip(&got)
                    .flat_map(|(a, b)| a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)))
                    .max()
                    .unwrap();
                assert!(
                    worst <= 2,
                    "{mode:?} {dims:?} orientation {value}: off by {worst}"
                );
            }
        }
    }
    // Hard edges: every quadrant's middle is its color, in every
    // orientation.
    for value in 1..=8 {
        let image = stored(value, w, h, |x, y| quadrant(x, y, w, h));
        let mut buffer = render(Source::Owned(image), look(Mode::Stretch), (80, 48)).unwrap();
        let got = pixels(&mut buffer);
        for (x, y) in [(20, 12), (60, 12), (20, 36), (60, 36)] {
            let want = quadrant(x / 2, y / 2, w, h);
            assert!(
                close(got[(y * 80 + x) as usize], want, 2),
                "orientation {value} at ({x},{y})"
            );
        }
    }
}

#[test]
fn fill_covers_and_fit_letterboxes() {
    let (w, h) = (40, 20);
    let wide = stored(1, w, h, |x, y| quadrant(x, y, w, h));
    // Fill onto a square: the middle 20×20, so all four quadrants meet at
    // the centre and the corners keep their colors.
    let mut buffer = render(Source::Borrowed(&wide), look(Mode::Fill), (100, 100)).unwrap();
    let got = pixels(&mut buffer);
    assert!(close(got[5 * 100 + 5], [255, 0, 0], 2));
    assert!(close(got[5 * 100 + 94], [0, 255, 0], 2));
    assert!(close(got[94 * 100 + 5], [0, 0, 255], 2));
    assert!(close(got[94 * 100 + 94], [255, 255, 255], 2));
    // Fit onto a square: a 100×50 band in the middle, fill above and below.
    let mut buffer = render(Source::Borrowed(&wide), look(Mode::Fit), (100, 100)).unwrap();
    let got = pixels(&mut buffer);
    for x in 0..100 {
        for y in (0..25).chain(75..100) {
            assert_eq!(got[y * 100 + x], [1, 2, 3], "bar at ({x},{y})");
        }
    }
    assert!(close(got[30 * 100 + 5], [255, 0, 0], 2));
    assert!(close(got[69 * 100 + 94], [255, 255, 255], 2));
}

#[test]
fn center_and_tile_are_exact_copies() {
    let (w, h) = (6, 4);
    let numbered = |x: u32, y: u32| [x as u8 * 10, y as u8 * 10, 7];
    for value in 1..=8 {
        let image = stored(value, w, h, numbered);
        let mut buffer = render(Source::Borrowed(&image), look(Mode::Center), (10, 8)).unwrap();
        let got = pixels(&mut buffer);
        for y in 0..8 {
            for x in 0..10 {
                let want = if (2..8).contains(&x) && (2..6).contains(&y) {
                    numbered(x - 2, y - 2)
                } else {
                    [1, 2, 3]
                };
                assert_eq!(got[(y * 10 + x) as usize], want, "center {value} ({x},{y})");
            }
        }
        let mut buffer = render(Source::Owned(image), look(Mode::Tile), (13, 9)).unwrap();
        let got = pixels(&mut buffer);
        for y in 0..9 {
            for x in 0..13 {
                assert_eq!(
                    got[(y * 13 + x) as usize],
                    numbered(x % 6, y % 4),
                    "tile {value}"
                );
            }
        }
    }
    // Larger than the output: the middle shows.
    let image = stored(1, w, h, numbered);
    let mut buffer = render(Source::Borrowed(&image), look(Mode::Center), (2, 2)).unwrap();
    assert_eq!(
        pixels(&mut buffer),
        [
            numbered(2, 1),
            numbered(3, 1),
            numbered(2, 2),
            numbered(3, 2)
        ]
    );
}

#[test]
fn owned_and_borrowed_render_the_same() {
    let (w, h) = (50, 31);
    let image = || {
        stored(6, w, h, |x, y| {
            [(x * 5) as u8, (y * 8) as u8, ((x + y) * 3) as u8]
        })
    };
    for mode in Mode::ALL {
        for dims in [(17, 40), (100, 30)] {
            let borrowed_source = image();
            let mut a = render(Source::Borrowed(&borrowed_source), look(mode), dims).unwrap();
            let mut b = render(Source::Owned(image()), look(mode), dims).unwrap();
            assert_eq!(pixels(&mut a), pixels(&mut b), "{mode:?} {dims:?}");
        }
    }
}

#[test]
fn one_pixel_images_and_outputs_render() {
    for mode in Mode::ALL {
        for (image, dims) in [
            ((1, 1), (7, 5)),
            ((9, 1), (1, 1)),
            ((1, 9), (3, 4)),
            ((1, 1), (1, 1)),
        ] {
            let decoded = stored(1, image.0, image.1, |_, _| [9, 8, 7]);
            let mut buffer = render(Source::Owned(decoded), look(mode), dims).unwrap();
            let got = pixels(&mut buffer);
            assert_eq!(got.len(), (dims.0 * dims.1) as usize);
            assert!(
                got.iter().any(|p| close(*p, [9, 8, 7], 1)),
                "{mode:?} {image:?} {dims:?}"
            );
        }
    }
}

#[test]
fn empty_and_inconsistent_inputs_are_errors() {
    let image = stored(1, 4, 4, |_, _| [0, 0, 0]);
    // A size `wl_shm` cannot take, refused before anything is scaled.
    for dims in [(0, 5), (5, 0), (70_000, 70_000)] {
        assert!(matches!(
            render(Source::Borrowed(&image), look(Mode::Fill), dims),
            Err(RenderError::Shm(_))
        ));
    }
    let lying = Decoded {
        rgb: vec![0; 10],
        width: 4,
        height: 4,
        orientation: Orientation::NORMAL,
    };
    assert!(matches!(
        render(Source::Owned(lying), look(Mode::Fill), (8, 8)),
        Err(RenderError::Empty)
    ));
    // Past what wl_shm can take: the buffer refuses, no panic.
    assert!(matches!(
        render(
            Source::Borrowed(&image),
            look(Mode::Tile),
            (u32::MAX, u32::MAX)
        ),
        Err(RenderError::Shm(_))
    ));
}

#[test]
fn crop_in_place_keeps_exactly_the_rectangle() {
    let (w, h) = (7, 5);
    let mut rgb: Vec<u8> = (0..w * h).flat_map(|i| [i as u8, 0, 0]).collect();
    let crop = Rect {
        x: 2,
        y: 1,
        width: 3,
        height: 2,
    };
    crop_in_place(&mut rgb, w, h, crop).unwrap();
    let kept: Vec<u8> = rgb.chunks_exact(3).map(|p| p[0]).collect();
    assert_eq!(kept, [9, 10, 11, 16, 17, 18]);
    let mut rgb = vec![0; 7 * 5 * 3];
    for bad in [
        Rect {
            x: 5,
            y: 0,
            width: 3,
            height: 1,
        },
        Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 1,
        },
        Rect {
            x: 0,
            y: 4,
            width: 1,
            height: 2,
        },
        Rect {
            x: u32::MAX,
            y: 0,
            width: 2,
            height: 1,
        },
    ] {
        assert!(crop_in_place(&mut rgb, 7, 5, bad).is_err(), "{bad:?}");
    }
}

/// Every frame renders like its own still: the animation's buffers hold
/// each frame's pixels in order (red, green, then green, red).
#[test]
fn animated_renders_every_frame_like_its_still() {
    use std::io::Cursor;

    use super::render_animated;
    use crate::image::animated::decode_animated;
    use crate::image::samples;

    let animated = decode_animated(&mut Cursor::new(samples::gif_two_frame()), FILL)
        .unwrap()
        .expect("two frames");
    let mut buffers = render_animated(&animated, look(Mode::Stretch), (2, 1)).unwrap();
    assert_eq!(buffers.len(), 2);
    assert_eq!(pixels(&mut buffers[0]), [[255, 0, 0], [0, 255, 0]]);
    assert_eq!(pixels(&mut buffers[1]), [[0, 255, 0], [255, 0, 0]]);
}
