use std::path::PathBuf;

use super::{Error, MAX_FILE, MAX_SIDE, decode, load};

/// A PNG with `data` (raw rows, no filter bytes) as the encoder writes it.
fn encode(
    width: u32,
    height: u32,
    color: png::ColorType,
    depth: png::BitDepth,
    data: &[u8],
    tweak: impl FnOnce(&mut png::Encoder<&mut Vec<u8>>),
) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(color);
        encoder.set_depth(depth);
        tweak(&mut encoder);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(data).unwrap();
    }
    out
}

fn rgba(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut data = Vec::new();
    for y in 0..height {
        for x in 0..width {
            data.extend_from_slice(&pixel(x, y));
        }
    }
    encode(
        width,
        height,
        png::ColorType::Rgba,
        png::BitDepth::Eight,
        &data,
        |_| {},
    )
}

/// Premultiplied `b, g, r, a` of pixel `(x, y)`.
fn pixel(image: &super::Image, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * image.width + x) * 4) as usize;
    image.pixels[at..at + 4].try_into().unwrap()
}

fn scaled(image: &super::Image, side: u32) -> Vec<u8> {
    let mut out = vec![0xaau8; (side * side * 4) as usize];
    image.scale_into(side, &mut out);
    out
}

fn px(out: &[u8], side: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * side + x) * 4) as usize;
    out[at..at + 4].try_into().unwrap()
}

/// CRC-32 (IEEE), for building chunks by hand.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
    out
}

fn ihdr(width: u32, height: u32, depth: u8, color: u8) -> Vec<u8> {
    let mut data = width.to_be_bytes().to_vec();
    data.extend_from_slice(&height.to_be_bytes());
    data.extend_from_slice(&[depth, color, 0, 0, 0]);
    chunk(b"IHDR", &data)
}

const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// The `IDAT` chunk payloads of a PNG, concatenated.
fn idat(png: &[u8]) -> Vec<u8> {
    let mut at = SIGNATURE.len();
    let mut out = Vec::new();
    while at + 8 <= png.len() {
        let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        if &png[at + 4..at + 8] == b"IDAT" {
            out.extend_from_slice(&png[at + 8..at + 8 + len]);
        }
        at += 12 + len;
    }
    out
}

#[test]
fn a_rgba_png_is_held_premultiplied_in_bgra_order() {
    let png = rgba(2, 1, |x, _| {
        if x == 0 {
            [200, 100, 50, 255]
        } else {
            [200, 100, 50, 128]
        }
    });
    let image = decode(&png).unwrap();
    assert_eq!((image.width, image.height), (2, 1));
    assert_eq!(pixel(&image, 0, 0), [50, 100, 200, 255]);
    // 128 / 255 of each channel, rounded.
    assert_eq!(pixel(&image, 1, 0), [25, 50, 100, 128]);
}

#[test]
fn other_color_types_and_depths_become_eight_bit_rgba() {
    // Grayscale, 8 bits.
    let png = encode(
        2,
        1,
        png::ColorType::Grayscale,
        png::BitDepth::Eight,
        &[0, 255],
        |_| {},
    );
    let image = decode(&png).unwrap();
    assert_eq!(pixel(&image, 0, 0), [0, 0, 0, 255]);
    assert_eq!(pixel(&image, 1, 0), [255, 255, 255, 255]);
    // Grayscale with alpha.
    let png = encode(
        1,
        1,
        png::ColorType::GrayscaleAlpha,
        png::BitDepth::Eight,
        &[200, 128],
        |_| {},
    );
    assert_eq!(pixel(&decode(&png).unwrap(), 0, 0), [100, 100, 100, 128]);
    // RGB.
    let png = encode(
        1,
        1,
        png::ColorType::Rgb,
        png::BitDepth::Eight,
        &[1, 2, 3],
        |_| {},
    );
    assert_eq!(pixel(&decode(&png).unwrap(), 0, 0), [3, 2, 1, 255]);
    // 16 bits a channel keeps the high byte.
    let png = encode(
        1,
        1,
        png::ColorType::Rgba,
        png::BitDepth::Sixteen,
        &[0xff, 0x00, 0x80, 0x00, 0x00, 0x00, 0xff, 0xff],
        |_| {},
    );
    assert_eq!(pixel(&decode(&png).unwrap(), 0, 0), [0, 0x80, 0xff, 255]);
    // One bit a pixel, expanded: 1 is white.
    let png = encode(
        2,
        1,
        png::ColorType::Grayscale,
        png::BitDepth::One,
        &[0b0100_0000],
        |_| {},
    );
    let image = decode(&png).unwrap();
    assert_eq!(pixel(&image, 0, 0), [0, 0, 0, 255]);
    assert_eq!(pixel(&image, 1, 0), [255, 255, 255, 255]);
    // A palette with a transparent first entry.
    let png = encode(
        2,
        1,
        png::ColorType::Indexed,
        png::BitDepth::Eight,
        &[0, 1],
        |encoder| {
            encoder.set_palette(vec![255, 0, 0, 0, 255, 0]);
            encoder.set_trns(vec![0]);
        },
    );
    let image = decode(&png).unwrap();
    assert_eq!(pixel(&image, 0, 0), [0, 0, 0, 0]);
    assert_eq!(pixel(&image, 1, 0), [0, 255, 0, 255]);
}

#[test]
fn a_solid_image_stays_solid_at_any_size() {
    let image = decode(&rgba(8, 8, |_, _| [255, 0, 0, 255])).unwrap();
    for side in [1u32, 3, 4, 8, 13, 32] {
        let out = scaled(&image, side);
        for chunk in out.chunks_exact(4) {
            assert_eq!(chunk, [0, 0, 255, 255], "side {side}");
        }
    }
}

#[test]
fn shrinking_averages_every_source_pixel() {
    // A 2x2 checkerboard of black and white to one pixel: mid gray.
    let image = decode(&rgba(2, 2, |x, y| {
        if (x + y) % 2 == 0 {
            [255, 255, 255, 255]
        } else {
            [0, 0, 0, 255]
        }
    }))
    .unwrap();
    let out = scaled(&image, 1);
    assert!(out[0].abs_diff(128) <= 1, "{out:?}");
    assert_eq!(out[3], 255);
    // 16 -> 4 of a half black, half white image: the middle pair of
    // columns is white and black, and no source pixel is skipped (a point
    // sample of every fourth pixel could not tell a 1-pixel line).
    let line = decode(&rgba(16, 16, |x, _| {
        if x == 5 {
            [255, 255, 255, 255]
        } else {
            [0, 0, 0, 255]
        }
    }))
    .unwrap();
    let out = scaled(&line, 4);
    let total: u32 = (0..4).map(|x| u32::from(px(&out, 4, x, 0)[0])).sum();
    // 1/16 of a 4-wide row of white, spread over the row: 255 / 4.
    assert!(total.abs_diff(64) <= 4, "{total}: {out:?}");
}

#[test]
fn a_transparent_pixel_does_not_fringe_its_neighbors() {
    // Opaque white beside transparent black (color 0): premultiplied, the
    // blend is white at half alpha, so every channel equals alpha; a
    // straight-alpha blend would go gray.
    let image = decode(&rgba(8, 2, |x, _| {
        if x < 4 {
            [255, 255, 255, 255]
        } else {
            [0, 0, 0, 0]
        }
    }))
    .unwrap();
    for side in [3u32, 5, 8, 20] {
        let out = scaled(&image, side);
        for chunk in out.chunks_exact(4) {
            assert!(
                chunk[0].abs_diff(chunk[3]) <= 1 && chunk[1].abs_diff(chunk[3]) <= 1,
                "side {side}: {chunk:?}"
            );
        }
    }
}

#[test]
fn a_wide_image_keeps_its_aspect_ratio_centered() {
    let image = decode(&rgba(8, 4, |_, _| [255, 255, 255, 255])).unwrap();
    let out = scaled(&image, 8);
    for y in 0..8u32 {
        for x in 0..8u32 {
            let want = if (2..6).contains(&y) { 255 } else { 0 };
            assert_eq!(px(&out, 8, x, y)[3], want, "({x}, {y})");
        }
    }
    // A tall one, centered across.
    let image = decode(&rgba(4, 8, |_, _| [255, 255, 255, 255])).unwrap();
    let out = scaled(&image, 8);
    for y in 0..8u32 {
        for x in 0..8u32 {
            let want = if (2..6).contains(&x) { 255 } else { 0 };
            assert_eq!(px(&out, 8, x, y)[3], want, "({x}, {y})");
        }
    }
    // A one-pixel-wide image is at least one pixel wide when drawn.
    let image = decode(&rgba(1, 64, |_, _| [255, 255, 255, 255])).unwrap();
    let out = scaled(&image, 16);
    assert!((0..16).any(|x| px(&out, 16, x, 8)[3] > 0));
}

#[test]
fn scaling_is_safe_at_the_extremes() {
    let image = decode(&rgba(3, 5, |x, y| [x as u8 * 50, y as u8 * 40, 7, 255])).unwrap();
    // A side of 0 writes nothing; a buffer too short is untouched.
    image.scale_into(0, &mut []);
    let mut short = [9u8; 10];
    image.scale_into(4, &mut short);
    assert_eq!(short, [9u8; 10]);
    for side in [1u32, 2, 511, 512] {
        let out = scaled(&image, side);
        assert_eq!(out.len(), (side * side * 4) as usize);
    }
    // A largest image, down to the smallest and up to the largest sizes.
    let big = decode(&rgba(MAX_SIDE, 1024, |x, y| [x as u8, y as u8, 0, 255])).unwrap();
    for side in [1u32, 16, 512] {
        let out = scaled(&big, side);
        assert!(out.iter().skip(3).step_by(4).all(|&a| a >= 254 || a == 0));
    }
}

/// A scratch directory of its own.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir = std::env::temp_dir().join(format!(
            "scootbar-image-{}-{nanos}-{tag}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_file_loads_and_what_is_not_a_usable_file_says_why() {
    let scratch = Scratch::new("files");
    let good = scratch.file("good.png", &rgba(2, 2, |_, _| [1, 2, 3, 255]));
    assert_eq!(load(&good).unwrap().width, 2);
    let missing = scratch.0.join("missing.png");
    assert!(matches!(load(&missing), Err(Error::Io(_))));
    assert!(matches!(load(&scratch.0), Err(Error::NotRegular)));
    assert!(matches!(
        load(std::path::Path::new("/dev/zero")),
        Err(Error::NotRegular)
    ));
    let empty = scratch.file("empty.png", b"");
    assert!(matches!(load(&empty), Err(Error::Empty)));
    let junk = scratch.file("junk.png", b"this is not a png at all");
    assert!(matches!(load(&junk), Err(Error::Png(_))));
    // A FIFO with no writer is refused without blocking.
    let fifo = scratch.0.join("fifo");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
    )
    .unwrap();
    assert!(matches!(load(&fifo), Err(Error::NotRegular)));
    // A symlink loop is the kernel's refusal.
    let (a, b) = (scratch.0.join("a.png"), scratch.0.join("b.png"));
    std::os::unix::fs::symlink(&b, &a).unwrap();
    std::os::unix::fs::symlink(&a, &b).unwrap();
    assert!(matches!(load(&a), Err(Error::Io(_))));
    // A link to a real file is followed, like a font's.
    let link = scratch.0.join("link.png");
    std::os::unix::fs::symlink(&good, &link).unwrap();
    assert!(load(&link).is_ok());
    // Past the cap, by size alone (a sparse file costs nothing).
    let big = scratch.0.join("big.png");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(MAX_FILE + 1)
        .unwrap();
    assert!(matches!(load(&big), Err(Error::TooLarge(n)) if n == MAX_FILE + 1));
    // Every message names what is wrong.
    for error in [Error::NotRegular, Error::Empty, Error::Dimensions(0, 5)] {
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn a_truncated_png_is_an_error_at_every_length_never_a_panic() {
    let png = rgba(9, 7, |x, y| [x as u8 * 20, y as u8 * 30, 99, 255]);
    assert!(decode(&png).is_ok());
    // Cut anywhere before the end of the image data it is an error; a cut
    // inside the trailing `IEND` chunk leaves the whole picture, which is
    // decoded (and nothing panics either way).
    let iend = png.len() - 12;
    for end in 0..png.len() {
        let result = decode(&png[..end]);
        if end < iend {
            assert!(result.is_err(), "prefix of {end} bytes");
        }
    }
}

#[test]
fn a_bad_checksum_is_an_error() {
    let png = rgba(4, 4, |_, _| [9, 9, 9, 255]);
    // Flip a byte of the data chunk's CRC (the last four bytes before
    // IEND's 12), and of the header's.
    let iend = png.len() - 12;
    let mut bad = png.clone();
    bad[iend - 1] ^= 0xff;
    assert!(decode(&bad).is_err());
    let mut bad = png.clone();
    // The IHDR CRC ends at 8 + 8 + 13 + 4.
    bad[8 + 8 + 13 + 3] ^= 0x01;
    assert!(decode(&bad).is_err());
    // A flipped byte inside the compressed data.
    let mut bad = png.clone();
    bad[iend - 8] ^= 0x55;
    assert!(decode(&bad).is_err());
}

#[test]
fn an_image_past_the_bounds_is_refused_before_a_pixel_buffer_exists() {
    let end = chunk(b"IEND", &[]);
    for (w, h) in [
        (0, 10),
        (10, 0),
        (MAX_SIDE + 1, 1),
        (1, MAX_SIDE + 1),
        (100_000, 100_000),
        (u32::MAX >> 1, u32::MAX >> 1),
        (1024, 1025),
    ] {
        let mut png = SIGNATURE.to_vec();
        png.extend(ihdr(w, h, 8, 6));
        // The data is one empty compressed block: nothing like the size
        // the header declares.
        png.extend(chunk(
            b"IDAT",
            &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
        ));
        png.extend(&end);
        let error = decode(&png).unwrap_err();
        assert!(
            matches!(error, Error::Dimensions(..) | Error::Png(_)),
            "{w}x{h}: {error}"
        );
    }
    // The bounds' own edge is fine: 1024 x 1024 declared and present.
    let full = rgba(1024, 1024, |_, _| [0, 0, 0, 0]);
    assert_eq!(decode(&full).unwrap().width, 1024);
}

#[test]
fn compressed_data_far_past_the_declared_size_is_not_a_memory_bomb() {
    // 4096 x 4096 gray zeros compress to a few KiB; put that data behind
    // a header declaring 1 x 1.
    let zeros = vec![0u8; 4096 * 4096];
    let big = encode(
        4096,
        4096,
        png::ColorType::Grayscale,
        png::BitDepth::Eight,
        &zeros,
        |encoder| encoder.set_compression(png::Compression::High),
    );
    let data = idat(&big);
    assert!(data.len() < 100_000, "{} bytes of data", data.len());
    let mut png = SIGNATURE.to_vec();
    png.extend(ihdr(1, 1, 8, 0));
    png.extend(chunk(b"IDAT", &data));
    png.extend(chunk(b"IEND", &[]));
    // Refused or read as its one pixel, quickly, and no panic; the buffer
    // it makes is the header's, one pixel.
    match decode(&png) {
        Ok(image) => assert_eq!((image.width, image.height), (1, 1)),
        Err(Error::Png(_)) => {}
        Err(other) => panic!("{other}"),
    }
    // The declared size itself, at the header, is what the bound reads: a
    // 4096-wide image is refused outright.
    assert!(matches!(decode(&big), Err(Error::Dimensions(4096, 4096))));
}

#[test]
fn pixels_of_an_equal_picture_are_equal_whatever_the_id() {
    let a = decode(&rgba(2, 2, |_, _| [1, 2, 3, 255])).unwrap();
    let b = decode(&rgba(2, 2, |_, _| [1, 2, 3, 255])).unwrap();
    let c = decode(&rgba(2, 2, |_, _| [1, 2, 4, 255])).unwrap();
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_ne!(a.id(), b.id());
}
