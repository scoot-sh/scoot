//! The lock screen picture, stage by stage, on a synthetic frame: an
//! `#[ignore]`d test, run by hand in a release build:
//!
//! ```sh
//! cargo test --release -p scootbg --bin scootbg -- --ignored --nocapture \
//!     --test-threads 1 lockblur::bench
//! ```
//!
//! For each size, five runs of the three stages `blurred_png` is made of
//! (area average, box blur, PNG encode) and of the whole, in milliseconds,
//! with the picture's size in bytes, then [`solid_png`] (a color's picture,
//! made on the loop thread) at the same sizes.

use std::time::Instant;

use super::*;

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

/// A frame with large smooth areas and fine detail, like a photograph.
fn frame(width: u32, height: u32) -> Vec<u8> {
    let mut buffer = vec![0_u8; width as usize * height as usize * 4];
    let mut noise = 0x2545_F491_u32;
    for y in 0..height {
        for x in 0..width {
            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let grain = (noise >> 28) as u8;
            let at = (y as usize * width as usize + x as usize) * 4;
            buffer[at] = ((x * 255 / width) as u8).wrapping_add(grain);
            buffer[at + 1] = ((y * 255 / height) as u8).wrapping_add(grain);
            buffer[at + 2] = (((x + y) * 255 / (width + height)) as u8).wrapping_add(grain);
        }
    }
    buffer
}

#[test]
#[ignore = "a benchmark: run by hand in a release build, see the module docs"]
fn stages() {
    for (width, height) in [(1920, 1080), (2560, 1440), (3840, 2160)] {
        let buffer = frame(width, height);
        let stride = width as usize * 4;
        let (small_w, small_h) = small_dims(width, height).unwrap();
        for run in 0..5 {
            let t = Instant::now();
            let mut rgb = average(&buffer, (width, height), stride, (small_w, small_h)).unwrap();
            let average_ms = ms(t);
            let t = Instant::now();
            let radius = (small_w * SIGMA_PERMILLE / 1000).max(1) as usize;
            blur(&mut rgb, small_w as usize, small_h as usize, radius);
            let blur_ms = ms(t);
            let t = Instant::now();
            let png = encode(&rgb, small_w, small_h).unwrap();
            let encode_ms = ms(t);
            let t = Instant::now();
            let whole = blurred_png(&buffer, width, height, stride).unwrap();
            let whole_ms = ms(t);
            assert_eq!(png, whole);
            println!(
                "{width}x{height} -> {small_w}x{small_h} run {run}: average {average_ms:.2} ms, \
                 blur {blur_ms:.2} ms, encode {encode_ms:.2} ms; whole {whole_ms:.2} ms; \
                 png {} bytes",
                png.len()
            );
        }
    }
}

#[test]
#[ignore = "a benchmark: run by hand in a release build, see the module docs"]
fn solid() {
    let color = Color {
        r: 30,
        g: 60,
        b: 90,
    };
    for (width, height) in [(1920, 1080), (3840, 2160)] {
        for run in 0..5 {
            let t = Instant::now();
            let png = solid_png(color, width, height).unwrap();
            println!(
                "solid {width}x{height} run {run}: {:.3} ms, png {} bytes",
                ms(t),
                png.len()
            );
        }
    }
}
