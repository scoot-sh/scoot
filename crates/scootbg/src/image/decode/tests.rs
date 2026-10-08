use std::io::Cursor;

use super::{DecodeError, Decoded, MAX_PIXELS, decode, decode_file, to_rgb};
use crate::color::Color;
use crate::image::orientation::Orientation;
use crate::image::samples::{self, QUADRANT_COLORS};

const FILL: Color = Color { r: 0, g: 0, b: 0 };

fn decoded(bytes: &[u8]) -> Result<Decoded, DecodeError> {
    decode(Cursor::new(bytes), FILL)
}

fn pixel(image: &Decoded, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * image.width + x) * 3) as usize;
    [image.rgb[i], image.rgb[i + 1], image.rgb[i + 2]]
}

fn close(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 3)
}

fn assert_quadrants(image: &Decoded) {
    assert_eq!((image.width, image.height), (32, 16));
    assert_eq!(image.rgb.len(), 32 * 16 * 3);
    for (i, (x, y)) in [(4, 4), (20, 4), (4, 12), (20, 12)].into_iter().enumerate() {
        assert!(
            close(pixel(image, x, y), QUADRANT_COLORS[i]),
            "({x},{y}): {:?}",
            pixel(image, x, y)
        );
    }
}

#[test]
fn jpeg_baseline_and_progressive() {
    for bytes in [samples::QUADRANTS_JPEG, samples::PROGRESSIVE_JPEG] {
        let image = decoded(bytes).unwrap();
        assert_quadrants(&image);
        assert_eq!(image.orientation, Orientation::NORMAL);
    }
}

#[test]
fn grey_jpeg_becomes_rgb() {
    let image = decoded(samples::GRAY_JPEG).unwrap();
    assert_eq!(
        (image.width, image.height, image.rgb.len()),
        (8, 8, 8 * 8 * 3)
    );
    for p in image.rgb.chunks_exact(3) {
        assert!(close([p[0], p[1], p[2]], [128, 128, 128]), "{p:?}");
    }
}

#[test]
fn jpeg_exif_orientation_is_read_in_every_value() {
    for value in 1..=8 {
        let bytes = samples::jpeg_with_orientation(samples::QUADRANTS_JPEG, value);
        let image = decoded(&bytes).unwrap();
        assert_quadrants(&image);
        assert_eq!(image.orientation.value(), value as u8);
    }
}

#[test]
fn png_in_every_color_type() {
    use png::{BitDepth, ColorType};
    // 2×1: one opaque pixel, one half-transparent where there is alpha.
    type Case = (ColorType, BitDepth, Vec<u8>, [[u8; 3]; 2]);
    let cases: [Case; 5] = [
        (
            ColorType::Rgb,
            BitDepth::Eight,
            vec![10, 20, 30, 40, 50, 60],
            [[10, 20, 30], [40, 50, 60]],
        ),
        (
            ColorType::Rgba,
            BitDepth::Eight,
            vec![10, 20, 30, 255, 200, 100, 50, 128],
            [[10, 20, 30], [100, 50, 25]],
        ),
        (
            ColorType::Grayscale,
            BitDepth::Eight,
            vec![7, 250],
            [[7, 7, 7], [250, 250, 250]],
        ),
        (
            ColorType::GrayscaleAlpha,
            BitDepth::Eight,
            vec![7, 255, 200, 0],
            [[7, 7, 7], [0, 0, 0]],
        ),
        // 16-bit, big-endian samples: the high byte is kept.
        (
            ColorType::Rgb,
            BitDepth::Sixteen,
            vec![1, 0, 2, 0, 3, 0, 255, 255, 0, 0, 128, 1],
            [[1, 2, 3], [255, 0, 128]],
        ),
    ];
    for (color, depth, data, want) in cases {
        let bytes = samples::png(2, 1, color, depth, &data, None);
        let image = decoded(&bytes).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(
            [pixel(&image, 0, 0), pixel(&image, 1, 0)],
            want,
            "{color:?} {depth:?}"
        );
    }
    // A palette, and 1-bit grey: expanded.
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 3, 1);
        encoder.set_color(ColorType::Indexed);
        encoder.set_palette(vec![255, 0, 0, 0, 0, 255]);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[0, 1, 0]).unwrap();
    }
    let image = decoded(&out).unwrap();
    assert_eq!(image.rgb, [255, 0, 0, 0, 0, 255, 255, 0, 0]);
    let bytes = samples::png(
        8,
        1,
        ColorType::Grayscale,
        BitDepth::One,
        &[0b1010_0000],
        None,
    );
    let image = decoded(&bytes).unwrap();
    assert_eq!(pixel(&image, 0, 0), [255, 255, 255]);
    assert_eq!(pixel(&image, 1, 0), [0, 0, 0]);
}

#[test]
fn transparency_is_flattened_over_the_fill() {
    let bytes = samples::png(
        1,
        1,
        png::ColorType::Rgba,
        png::BitDepth::Eight,
        &[255, 255, 255, 0],
        None,
    );
    let over = |fill| decode(Cursor::new(&bytes), fill).unwrap().rgb;
    assert_eq!(
        over(Color {
            r: 16,
            g: 32,
            b: 48
        }),
        [16, 32, 48]
    );
    let webp = samples::webp(1, 1, &[0, 0, 0, 0], true, None);
    let image = decode(Cursor::new(&webp), Color { r: 1, g: 2, b: 3 }).unwrap();
    assert_eq!(image.rgb, [1, 2, 3]);
}

#[test]
fn png_and_webp_exif_orientation() {
    for value in [1, 3, 6, 8] {
        let bytes = samples::png(
            1,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[1, 2, 3],
            Some(value),
        );
        assert_eq!(
            decoded(&bytes).unwrap().orientation.value(),
            value as u8,
            "png"
        );
        let bytes = samples::webp(1, 1, &[1, 2, 3], false, Some(value));
        assert_eq!(
            decoded(&bytes).unwrap().orientation.value(),
            value as u8,
            "webp"
        );
    }
}

#[test]
fn webp_lossless_rgb_and_rgba() {
    let data: Vec<u8> = (0..6 * 4).flat_map(|i| [i as u8, 100, 200]).collect();
    let image = decoded(&samples::webp(6, 4, &data, false, None)).unwrap();
    assert_eq!((image.width, image.height), (6, 4));
    assert_eq!(image.rgb, data);
    let rgba: Vec<u8> = (0..6 * 4).flat_map(|i| [i as u8, 100, 200, 255]).collect();
    let image = decoded(&samples::webp(6, 4, &rgba, true, None)).unwrap();
    assert_eq!(image.rgb, data);
}

#[test]
fn what_is_not_an_image_says_so() {
    for bytes in [
        &b""[..],
        b"\x89PN",
        b"GIF89",
        b"RIFF\x00\x00\x00\x00WAVE",
        b"BM\x00\x00\x00\x00\x00\x00",
        b"hello, world, this is text",
        b"\xff\xd8",
    ] {
        assert!(
            matches!(decoded(bytes), Err(DecodeError::NotAnImage)),
            "{bytes:?}"
        );
    }
}

/// Every truncation of every format is an error, never a panic, and
/// never a half-drawn picture: a file cut short is reported. The one cut
/// that may decode is a PNG missing only (part of) its closing `IEND`
/// chunk, whose picture is whole: then it must be exactly the whole
/// file's.
#[test]
fn truncated_files_are_errors() {
    let data: Vec<u8> = (0..16 * 16)
        .flat_map(|i| [i as u8, (i * 7) as u8, (i * 13) as u8])
        .collect();
    let files = [
        ("jpeg", samples::QUADRANTS_JPEG.to_vec()),
        ("progressive", samples::PROGRESSIVE_JPEG.to_vec()),
        (
            "png",
            samples::png(
                16,
                16,
                png::ColorType::Rgb,
                png::BitDepth::Eight,
                &data,
                None,
            ),
        ),
        ("webp", samples::webp(16, 16, &data, false, None)),
    ];
    for (name, bytes) in files {
        let whole = decoded(&bytes).unwrap();
        for len in 0..bytes.len() {
            if let Ok(image) = decoded(&bytes[..len]) {
                let iend = name == "png" && len >= bytes.len() - 12;
                assert!(iend, "{name} cut at {len} of {} decoded", bytes.len());
                assert_eq!(image.rgb, whole.rgb, "{name} cut at {len}");
            }
        }
    }
}

#[test]
fn corrupt_bytes_never_panic() {
    let data: Vec<u8> = (0..16 * 16 * 3).map(|i| (i * 31) as u8).collect();
    let files = [
        samples::QUADRANTS_JPEG.to_vec(),
        samples::PROGRESSIVE_JPEG.to_vec(),
        samples::png(
            16,
            16,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &data,
            None,
        ),
        samples::webp(16, 16, &data, false, None),
    ];
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for bytes in files {
        for _ in 0..300 {
            let mut broken = bytes.clone();
            for _ in 0..1 + next() % 4 {
                let at = (next() as usize) % broken.len();
                broken[at] = next() as u8;
            }
            if let Ok(image) = decoded(&broken) {
                assert_eq!(image.rgb.len(), (image.width * image.height * 3) as usize);
            }
        }
    }
}

/// A few bytes claiming an enormous image are refused from the header,
/// before the pixels are allocated.
#[test]
fn decompression_bombs_are_refused_from_the_header() {
    // PNG: a header of 100000×100000, and no data at all.
    let mut bomb = Vec::new();
    {
        let encoder = png::Encoder::new(&mut bomb, 100_000, 100_000);
        let writer = encoder.write_header().unwrap();
        drop(writer);
    }
    match decoded(&bomb) {
        Err(DecodeError::TooLarge { width, height }) => {
            assert_eq!((width, height), (100_000, 100_000))
        }
        other => panic!("{other:?}"),
    }
    // JPEG: the fixture's frame header (SOF) rewritten to 65535×65535.
    let mut jpeg = samples::QUADRANTS_JPEG.to_vec();
    let sof = jpeg
        .windows(2)
        .position(|w| w[0] == 0xff && (w[1] == 0xc0 || w[1] == 0xc2))
        .unwrap();
    jpeg[sof + 5..sof + 9].copy_from_slice(&[0xff, 0xff, 0xff, 0xff]);
    assert!(matches!(decoded(&jpeg), Err(DecodeError::TooLarge { .. })));
    // In the budget (16384×16384) but with the fixture's 312 bytes of
    // data: refused from the header too, for holding less than such a
    // JPEG can. (It used to decode as 805 MB of grey, and show.)
    jpeg[sof + 5..sof + 9].copy_from_slice(&[0x40, 0x00, 0x40, 0x00]);
    match decoded(&jpeg) {
        Err(DecodeError::Corrupt(why)) => assert!(why.contains("16384x16384"), "{why}"),
        other => panic!("{other:?}"),
    }
    // Just past the budget, and at it, by the arithmetic alone.
    assert!(super::budget(16385, 16384).is_err());
    assert!(super::budget(u64::MAX, u64::MAX).is_err());
    assert!(super::budget(1 << 32, 1).is_err());
    assert_eq!(super::budget(16384, 16384).unwrap(), (16384, 16384));
    assert_eq!(super::budget(MAX_PIXELS, 1).unwrap(), (1 << 28, 1));
    assert!(matches!(super::budget(0, 5), Err(DecodeError::Corrupt(_))));
}

#[test]
fn files_that_are_not_images_or_not_there() {
    let dir = std::env::temp_dir().join(format!("sbg-decode-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(matches!(
        decode_file(&dir.join("missing.png"), FILL),
        Err(DecodeError::NotFound)
    ));
    assert!(matches!(
        decode_file(&dir.join("missing/x.png"), FILL),
        Err(DecodeError::NotFound)
    ));
    assert!(matches!(
        decode_file(&dir, FILL),
        Err(DecodeError::NotAFile)
    ));
    // A FIFO with no writer: refused at once, not waited on.
    let fifo = dir.join("fifo");
    let _ = std::fs::remove_file(&fifo);
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    assert!(matches!(
        decode_file(&fifo, FILL),
        Err(DecodeError::NotAFile)
    ));
    let text = dir.join("text.png");
    std::fs::write(&text, "not a png").unwrap();
    assert!(matches!(
        decode_file(&text, FILL),
        Err(DecodeError::NotAnImage)
    ));
    let real = dir.join("real.jpg");
    std::fs::write(&real, samples::QUADRANTS_JPEG).unwrap();
    assert_quadrants(&decode_file(&real, FILL).unwrap());
    assert!(matches!(
        decode_file(std::path::Path::new("/dev/null"), FILL),
        Err(DecodeError::NotAFile)
    ));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn to_rgb_converts_in_place_and_checks_sizes() {
    let fill = Color {
        r: 0,
        g: 100,
        b: 200,
    };
    let mut grey = vec![5, 6, 0, 0, 0, 0];
    to_rgb(&mut grey, 2, 1, fill).unwrap();
    assert_eq!(grey, [5, 5, 5, 6, 6, 6]);
    let mut grey_alpha = vec![10, 255, 20, 0, 0, 0];
    to_rgb(&mut grey_alpha, 2, 2, fill).unwrap();
    assert_eq!(grey_alpha, [10, 10, 10, 0, 100, 200]);
    let mut rgba = vec![1, 2, 3, 255, 4, 5, 6, 0];
    to_rgb(&mut rgba, 2, 4, fill).unwrap();
    assert_eq!(rgba, [1, 2, 3, 0, 100, 200]);
    // Too short for what it claims, or a channel count that is not one.
    assert!(to_rgb(&mut vec![0; 5], 2, 1, fill).is_err());
    assert!(to_rgb(&mut vec![0; 7], 2, 4, fill).is_err());
    assert!(to_rgb(&mut vec![0; 64], 2, 5, fill).is_err());
    assert!(to_rgb(&mut vec![0; 64], 2, 0, fill).is_err());
    assert!(to_rgb(&mut vec![0; 64], usize::MAX, 3, fill).is_err());
}

/// Data after the end-of-image marker (a phone's motion-photo video, a
/// trailer, padding) is not truncation.
#[test]
fn jpeg_with_data_after_its_end_decodes() {
    for bytes in [samples::QUADRANTS_JPEG, samples::PROGRESSIVE_JPEG] {
        let mut trailing = bytes.to_vec();
        trailing.extend(std::iter::repeat_n(0x55, 100_000));
        assert_quadrants(&decoded(&trailing).unwrap());
        let mut appended = bytes.to_vec();
        appended.extend_from_slice(samples::GRAY_JPEG);
        appended.extend(std::iter::repeat_n(0xff, 5000));
        assert_quadrants(&decoded(&appended).unwrap());
    }
}

#[test]
fn the_jpeg_size_bound_is_one_bit_per_block() {
    use super::jpeg_min_len;
    assert_eq!(jpeg_min_len(8, 8), 1);
    assert_eq!(jpeg_min_len(1, 1), 1);
    assert_eq!(jpeg_min_len(64, 8), 1);
    assert_eq!(jpeg_min_len(72, 8), 2);
    assert_eq!(jpeg_min_len(16384, 16384), 524_288);
    assert_eq!(jpeg_min_len(6000, 4000), 46_875);
    assert_eq!(jpeg_min_len(u32::MAX, u32::MAX), 1 << 55, "no overflow");
    // Every fixture is well above it.
    for bytes in [
        samples::QUADRANTS_JPEG,
        samples::PROGRESSIVE_JPEG,
        samples::GRAY_JPEG,
    ] {
        let image = decoded(bytes).unwrap();
        assert!(bytes.len() as u64 >= jpeg_min_len(image.width, image.height));
    }
}

#[test]
fn gif_shows_its_first_frame() {
    let image = decoded(&samples::gif_two_frame()).unwrap();
    assert_eq!((image.width, image.height), (2, 1));
    assert_eq!(image.orientation, Orientation::NORMAL);
    // The first frame (red, green), not the second (green, red).
    assert_eq!(image.rgb, vec![255, 0, 0, 0, 255, 0]);
}
