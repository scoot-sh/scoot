//! The image contract: odd sizes refused, honest ones downscaled.

use scoot_dbus::proto::{Reader, Writer};

use super::{MAX_ICON_SIDE, MAX_IMAGE_SIDE, downscale, fuzz_check, read, validate};

/// Marshals one struct value's bytes: what the hint carries. The
/// writer is capped past the biggest seed (the downscale test's image
/// is 2 MB of pixels); the cap is the tests', not the protocol's.
fn struct_bytes(
    width: i32,
    height: i32,
    rowstride: i32,
    alpha: bool,
    bits: i32,
    channels: i32,
    pixels: &[u8],
) -> Vec<u8> {
    let mut body = Writer::with_cap(8 * 1024 * 1024);
    assert!(body.open_struct());
    body.i32(width);
    body.i32(height);
    body.i32(rowstride);
    body.boolean(alpha);
    body.i32(bits);
    body.i32(channels);
    let cookie = body.open_array(1).unwrap();
    body.raw(pixels);
    body.close_array(cookie);
    body.close_struct();
    body.take_body().unwrap()
}

fn solid(width: i32, height: i32, channels: i32, byte: u8) -> Vec<u8> {
    let rowstride = width * channels;
    vec![byte; (rowstride * height) as usize]
}

#[test]
fn a_small_image_converts_without_scaling() {
    let pixels = solid(2, 2, 4, 0x11);
    let bytes = struct_bytes(2, 2, 8, true, 8, 4, &pixels);
    let mut reader = Reader::le(&bytes);
    let raw = read(&mut reader).unwrap();
    let image = downscale(&validate(&raw).unwrap());
    assert_eq!((image.width, image.height), (2, 2));
    assert_eq!(image.pixels, vec![0x11; 16]);
}

#[test]
fn three_channels_gain_opaque_alpha() {
    let pixels = solid(1, 1, 3, 0x40);
    let bytes = struct_bytes(1, 1, 3, false, 8, 3, &pixels);
    let mut reader = Reader::le(&bytes);
    let raw = read(&mut reader).unwrap();
    let image = downscale(&validate(&raw).unwrap());
    assert_eq!(image.pixels, vec![0x40, 0x40, 0x40, 255]);
}

#[test]
fn a_large_image_downscales_within_the_bound() {
    let pixels = solid(1000, 500, 4, 0x80);
    let bytes = struct_bytes(1000, 500, 4000, true, 8, 4, &pixels);
    let mut reader = Reader::le(&bytes);
    let raw = read(&mut reader).unwrap();
    let image = downscale(&validate(&raw).unwrap());
    assert!(image.width <= MAX_ICON_SIDE && image.height <= MAX_ICON_SIDE);
    assert_eq!((image.width, image.height), (128, 64));
    assert_eq!(image.pixels.len(), 128 * 64 * 4);
    assert!(image.pixels.iter().all(|byte| *byte == 0x80));
}

#[test]
fn padding_in_the_rowstride_is_fine() {
    let mut pixels = vec![0u8; 3 * 4 + 2];
    pixels[0..3].copy_from_slice(&[9, 9, 9]);
    let bytes = struct_bytes(1, 1, 5, false, 8, 3, &pixels);
    let mut reader = Reader::le(&bytes);
    let raw = read(&mut reader).unwrap();
    assert!(validate(&raw).is_some());
}

#[test]
fn odd_shapes_are_no_image() {
    // Zero and negative sides, past-2048 sides, wrong bits, channels
    // disagreeing with alpha, a short rowstride, short pixels.
    let pixels = solid(4, 4, 4, 1);
    let cases = [
        struct_bytes(0, 4, 16, true, 8, 4, &pixels),
        struct_bytes(4, 0, 16, true, 8, 4, &pixels),
        struct_bytes(-4, 4, 16, true, 8, 4, &pixels),
        struct_bytes(MAX_IMAGE_SIDE + 1, 4, 16, true, 8, 4, &pixels),
        struct_bytes(4, 4, 16, true, 16, 4, &pixels),
        struct_bytes(4, 4, 16, false, 8, 4, &pixels),
        struct_bytes(4, 4, 16, true, 8, 3, &pixels),
        struct_bytes(4, 4, 16, true, 8, 2, &pixels),
        struct_bytes(4, 4, 15, true, 8, 4, &pixels),
        struct_bytes(4, 4, 16, true, 8, 4, &pixels[..15]),
        struct_bytes(4, 4, -1, true, 8, 4, &pixels),
    ];
    for bytes in &cases {
        let mut reader = Reader::le(bytes);
        match read(&mut reader) {
            Err(()) => {}
            Ok(raw) => assert!(validate(&raw).is_none(), "an odd shape validated"),
        }
    }
    // Trailing bytes past the struct are misshapen too.
    let mut trailing = struct_bytes(2, 2, 8, true, 8, 4, &solid(2, 2, 4, 1));
    trailing.push(0);
    let mut reader = Reader::le(&trailing);
    assert!(read(&mut reader).is_err());
}

/// Reverting the agreement check (accepting 3 channels with alpha) leaves
/// the odd-shape row failing: the pin is load-bearing.
#[test]
fn the_check_spots_seeds() {
    let pixels = solid(2, 2, 4, 7);
    assert!(fuzz_check(&struct_bytes(2, 2, 8, true, 8, 4, &pixels)));
    assert!(!fuzz_check(b"\x01\x02"));
}

#[test]
fn the_committed_corpus_replays_on_stable() {
    // Refusal seeds included: the check's own assertions pin the
    // contract, the replay pins no-panic survival.
    crate::replay_corpus("image-data", false, fuzz_check);
}
