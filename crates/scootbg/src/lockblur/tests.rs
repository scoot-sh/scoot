use super::*;

/// Decodes a PNG made here: its size and RGB bytes.
fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgb);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    buffer.truncate(info.buffer_size());
    (info.width, info.height, buffer)
}

/// An `XRGB8888` buffer (B, G, R, unused) from a per-pixel function.
fn xrgb(width: u32, height: u32, stride: usize, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let mut buffer = vec![0xEE; stride * height as usize];
    for y in 0..height {
        for x in 0..width {
            let [r, g, b] = pixel(x, y);
            let at = y as usize * stride + x as usize * 4;
            buffer[at..at + 4].copy_from_slice(&[b, g, r, 0]);
        }
    }
    buffer
}

#[test]
fn the_small_image_keeps_the_aspect_and_the_long_side_bound() {
    assert_eq!(small_dims(1920, 1080), Some((320, 180)));
    assert_eq!(small_dims(3840, 2160), Some((320, 180)));
    assert_eq!(small_dims(1080, 1920), Some((180, 320)));
    // Already small: unchanged.
    assert_eq!(small_dims(100, 50), Some((100, 50)));
    assert_eq!(small_dims(1, 1), Some((1, 1)));
    // A sliver keeps at least one pixel on its short side.
    assert_eq!(small_dims(10_000, 1), Some((320, 1)));
    assert_eq!(small_dims(1, 10_000), Some((1, 320)));
    assert_eq!(small_dims(MAX_SIDE, MAX_SIDE), Some((320, 320)));
}

#[test]
fn a_zero_or_absurd_size_has_no_image() {
    assert_eq!(small_dims(0, 10), None);
    assert_eq!(small_dims(10, 0), None);
    assert_eq!(small_dims(MAX_SIDE + 1, 1), None);
    assert_eq!(small_dims(1, MAX_SIDE + 1), None);
    assert_eq!(
        blurred_png(&[], 0, 0, 0),
        Err(BlurError::Size(0, 0)),
        "no arithmetic before the check"
    );
    assert_eq!(
        blurred_png(&[], u32::MAX, u32::MAX, usize::MAX),
        Err(BlurError::Size(u32::MAX, u32::MAX))
    );
}

#[test]
fn a_solid_buffer_stays_exactly_that_color() {
    for (w, h) in [(1920, 1080), (641, 359), (7, 3), (1, 1), (320, 1)] {
        let buffer = xrgb(w, h, w as usize * 4, |_, _| [12, 200, 77]);
        let png = blurred_png(&buffer, w, h, w as usize * 4).unwrap();
        let (sw, sh, rgb) = decode(&png);
        assert_eq!(Some((sw, sh)), small_dims(w, h));
        assert!(
            rgb.as_chunks::<3>().0.iter().all(|p| *p == [12, 200, 77]),
            "{w}x{h}: rounding must not drift a flat color"
        );
    }
}

#[test]
fn a_hard_edge_becomes_a_smooth_ramp_and_the_mean_is_kept() {
    let (w, h) = (1280, 720);
    // Left half black, right half white.
    let buffer = xrgb(w, h, w as usize * 4, |x, _| {
        if x < w / 2 { [0; 3] } else { [255; 3] }
    });
    let (sw, sh, rgb) = decode(&blurred_png(&buffer, w, h, w as usize * 4).unwrap());
    let row = &rgb[(sh as usize / 2) * sw as usize * 3..][..sw as usize * 3];
    let red: Vec<u8> = row.as_chunks::<3>().0.iter().map(|p| p[0]).collect();
    // Non-decreasing across the row: no ringing.
    assert!(red.windows(2).all(|pair| pair[0] <= pair[1]), "{red:?}");
    // The edge is soft: many distinct levels, not two.
    let mut levels = red.clone();
    levels.dedup();
    assert!(levels.len() > 20, "{} levels", levels.len());
    // Far from the edge the colors are still themselves; at the clamped
    // borders they have not faded toward the other side more than a blur
    // that wide allows.
    assert!(red[0] < 20 && red[red.len() - 1] > 235, "{red:?}");
    // The edge itself is mid-gray.
    let middle = red[red.len() / 2];
    assert!((100..=155).contains(&middle), "{middle}");
    // Rows are alike: the image varies along x only.
    let first_row = &rgb[..sw as usize * 3];
    let last_row = &rgb[(sh as usize - 1) * sw as usize * 3..];
    assert_eq!(first_row, last_row);
}

#[test]
fn channels_are_not_swapped() {
    let buffer = xrgb(64, 64, 256, |_, _| [255, 0, 0]);
    let (_, _, rgb) = decode(&blurred_png(&buffer, 64, 64, 256).unwrap());
    assert_eq!(&rgb[..3], [255, 0, 0], "red stays red");
}

#[test]
fn row_padding_is_ignored() {
    let (w, h) = (500, 300);
    let pixel = |x: u32, y: u32| [(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8];
    let tight = blurred_png(&xrgb(w, h, w as usize * 4, pixel), w, h, w as usize * 4).unwrap();
    let padded = blurred_png(
        &xrgb(w, h, w as usize * 4 + 64, pixel),
        w,
        h,
        w as usize * 4 + 64,
    )
    .unwrap();
    assert_eq!(tight, padded);
}

#[test]
fn a_buffer_shorter_than_its_size_says_is_an_error_not_a_panic() {
    let full = xrgb(100, 100, 400, |_, _| [1, 2, 3]);
    assert_eq!(
        blurred_png(&full[..full.len() - 1], 100, 100, 400),
        Err(BlurError::Short)
    );
    assert_eq!(blurred_png(&[], 100, 100, 400), Err(BlurError::Short));
    // A stride under one row.
    assert_eq!(blurred_png(&full, 100, 100, 399), Err(BlurError::Short));
    // A stride that overflows the size arithmetic.
    assert_eq!(
        blurred_png(&full, 100, 100, usize::MAX),
        Err(BlurError::Short)
    );
    // The last row needs no padding after it.
    let trimmed = xrgb(100, 100, 480, |_, _| [1, 2, 3]);
    assert!(blurred_png(&trimmed[..480 * 99 + 400], 100, 100, 480).is_ok());
}

#[test]
fn slivers_and_the_largest_side_work() {
    // One pixel.
    let one = xrgb(1, 1, 4, |_, _| [9, 8, 7]);
    let (w, h, rgb) = decode(&blurred_png(&one, 1, 1, 4).unwrap());
    assert_eq!((w, h, &rgb[..]), (1, 1, &[9, 8, 7][..]));
    // A strip a tall output would never be, and the bound itself.
    for (w, h) in [(MAX_SIDE, 1), (1, MAX_SIDE), (2, 3)] {
        let stride = w as usize * 4;
        let buffer = xrgb(w, h, stride, |x, y| [(x % 251) as u8, (y % 251) as u8, 5]);
        let (sw, sh, _) = decode(&blurred_png(&buffer, w, h, stride).unwrap());
        assert_eq!(Some((sw, sh)), small_dims(w, h), "{w}x{h}");
    }
}

#[test]
fn a_solid_color_picture_is_that_color_at_the_small_size() {
    let color = Color {
        r: 30,
        g: 60,
        b: 90,
    };
    let (w, h, rgb) = decode(&solid_png(color, 2560, 1440).unwrap());
    assert_eq!((w, h), (320, 180));
    assert!(rgb.as_chunks::<3>().0.iter().all(|p| *p == [30, 60, 90]));
    assert_eq!(solid_png(color, 0, 5), Err(BlurError::Size(0, 5)));
    assert_eq!(
        solid_png(color, MAX_SIDE + 1, 5),
        Err(BlurError::Size(MAX_SIDE + 1, 5))
    );
}

#[test]
fn the_picture_is_small() {
    // A busy 4K frame still encodes to a small picture (a smooth one to a
    // few kilobytes), which is what is kept per image.
    let (w, h) = (3840, 2160);
    let buffer = xrgb(w, h, w as usize * 4, |x, y| {
        [(x / 15) as u8, (y / 8) as u8, ((x ^ y) / 20) as u8]
    });
    let png = blurred_png(&buffer, w, h, w as usize * 4).unwrap();
    assert!(png.len() < 120_000, "{} bytes", png.len());
}

/// A tall box is sampled on a few of its rows, not all: against the exact
/// average of every pixel, a smooth picture differs by a few levels at
/// most, and a picture that varies with `y` alone keeps its rows in order.
#[test]
fn sampling_a_tall_box_stays_close_to_the_exact_average() {
    let (w, h) = (1600_u32, 1200_u32);
    let pixel = |x: u32, y: u32| {
        [
            (x * 255 / w) as u8,
            (y * 255 / h) as u8,
            ((x + y) * 255 / (w + h)) as u8,
        ]
    };
    let stride = w as usize * 4;
    let buffer = xrgb(w, h, stride, pixel);
    let (sw, sh) = small_dims(w, h).unwrap();
    assert!(
        h as usize / sh as usize > SAMPLE_ROWS,
        "the boxes are sampled"
    );
    let sampled = average(&buffer, (w, h), stride, (sw, sh)).unwrap();
    for sy in 0..sh as usize {
        for sx in 0..sw as usize {
            let (x0, x1) = (
                edge(sx, sw as usize, w as usize),
                edge(sx + 1, sw as usize, w as usize),
            );
            let (y0, y1) = (
                edge(sy, sh as usize, h as usize),
                edge(sy + 1, sh as usize, h as usize),
            );
            let mut sum = [0_u64; 3];
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = pixel(x as u32, y as u32);
                    for c in 0..3 {
                        sum[c] += u64::from(p[c]);
                    }
                }
            }
            let count = ((x1 - x0) * (y1 - y0)) as u64;
            for c in 0..3 {
                let exact = (sum[c] + count / 2) / count;
                let got = u64::from(sampled[(sy * sw as usize + sx) * 3 + c]);
                assert!(
                    exact.abs_diff(got) <= 3,
                    "({sx},{sy}) channel {c}: {got} against {exact}"
                );
            }
        }
    }
}
