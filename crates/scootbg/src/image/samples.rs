//! Test images, made in code where there is an encoder (`png`, `gif`,
//! and `image-webp`'s lossless one), else from the tiny fixtures in
//! `tests/fixtures/` (there is no JPEG encoder in the tree).

use super::exif::tests::block;

/// 32×16, four flat 16×8 quadrants: red, green / blue, white. Baseline,
/// 4:4:4, quality 100, so every quadrant's middle decodes within a step or
/// two of its color.
pub const QUADRANTS_JPEG: &[u8] = include_bytes!("../../tests/fixtures/quadrants.jpg");
/// The same, progressive.
pub const PROGRESSIVE_JPEG: &[u8] = include_bytes!("../../tests/fixtures/progressive.jpg");
/// 8×8 grey `#808080`, one channel.
pub const GRAY_JPEG: &[u8] = include_bytes!("../../tests/fixtures/gray.jpg");

/// The quadrants' colors, top-left, top-right, bottom-left, bottom-right.
pub const QUADRANT_COLORS: [[u8; 3]; 4] = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]];

/// An EXIF block recording `orientation` (big-endian, among other tags).
pub fn exif(orientation: u16) -> Vec<u8> {
    block(true, &[(0x010f, 2, 4, 0), (0x0112, 3, 1, orientation)])
}

/// `jpeg` with an APP1 EXIF segment recording `orientation` inserted after
/// its SOI marker, as a camera writes it.
pub fn jpeg_with_orientation(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let exif = exif(orientation);
    let len = u16::try_from(2 + 6 + exif.len()).unwrap();
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xff, 0xe1]);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(&exif);
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// A PNG of `data`, with an `eXIf` chunk recording `orientation` if given.
pub fn png(
    width: u32,
    height: u32,
    color: png::ColorType,
    depth: png::BitDepth,
    data: &[u8],
    orientation: Option<u16>,
) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(color);
        encoder.set_depth(depth);
        let mut writer = encoder.write_header().unwrap();
        if let Some(orientation) = orientation {
            writer
                .write_chunk(png::chunk::ChunkType(*b"eXIf"), &exif(orientation))
                .unwrap();
        }
        writer.write_image_data(data).unwrap();
        writer.finish().unwrap();
    }
    out
}

/// A single-frame GIF, 2×1, red and green: not an animation.
pub fn gif_single_frame() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut out, 2, 1, &[255, 0, 0, 0, 255, 0]).unwrap();
        let frame = gif::Frame {
            width: 2,
            height: 1,
            delay: 10,
            buffer: std::borrow::Cow::Borrowed(&[0, 1]),
            ..Default::default()
        };
        encoder.write_frame(&frame).unwrap();
    }
    out
}

/// A two-frame GIF, 2×1: red, green, then green, red, 100 ms a frame.
pub fn gif_two_frame() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut out, 2, 1, &[255, 0, 0, 0, 255, 0]).unwrap();
        encoder.set_repeat(gif::Repeat::Infinite).unwrap();
        for indices in [&[0u8, 1][..], &[1u8, 0][..]] {
            let frame = gif::Frame {
                width: 2,
                height: 1,
                delay: 10,
                buffer: std::borrow::Cow::Borrowed(indices),
                ..Default::default()
            };
            encoder.write_frame(&frame).unwrap();
        }
    }
    out
}

/// A GIF of `frames` 1×1 frames, alternating black and white, 100 ms
/// each: past the frame cap when asked for more than it keeps.
pub fn gif_many_frames(frames: usize) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut out, 1, 1, &[0, 0, 0, 255, 255, 255]).unwrap();
        encoder.set_repeat(gif::Repeat::Infinite).unwrap();
        for index in 0..frames {
            let indices = [(index % 2) as u8];
            let frame = gif::Frame {
                width: 1,
                height: 1,
                delay: 10,
                buffer: std::borrow::Cow::Borrowed(&indices),
                ..Default::default()
            };
            encoder.write_frame(&frame).unwrap();
        }
    }
    out
}

/// A lossless WebP of `data` (RGB, or RGBA with `alpha`), with EXIF
/// recording `orientation` if given.
pub fn webp(
    width: u32,
    height: u32,
    data: &[u8],
    alpha: bool,
    orientation: Option<u16>,
) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = image_webp::WebPEncoder::new(&mut out);
    if let Some(orientation) = orientation {
        encoder.set_exif_metadata(exif(orientation));
    }
    let color = if alpha {
        image_webp::ColorType::Rgba8
    } else {
        image_webp::ColorType::Rgb8
    };
    encoder.encode(data, width, height, color).unwrap();
    out
}
