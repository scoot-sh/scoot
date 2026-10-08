//! The fuzz target's entry point: arbitrary bytes through the whole image
//! path the daemon runs (decode, fit, crop, scale, pack), with every mode,
//! filter and orientation and odd buffer sizes.
//!
//! One function, [`whole_path`], shared by two callers: the `cargo fuzz`
//! target in `crates/scootbg/fuzz/` (which compiles this file into its own
//! crate by `#[path]`, since scootbg is a binary with no library to link
//! against) and the stable test in `fuzz/tests.rs` here, which replays the
//! committed corpus and every past crash through it on each `cargo test`.
//! So CI covers the corpus without a nightly toolchain, and a regression
//! file reproduces exactly as it did under the fuzzer.
//!
//! Only `crate::image` and `crate::color` are reached (as absolute paths,
//! which resolve in both crates): nothing here may use anything else of
//! scootbg's.
//!
//! **The guards are the daemon's own.** The decode refuses past
//! [`MAX_PIXELS`](crate::image::decode::MAX_PIXELS) from the header, and
//! [`render`](crate::image::render::render) refuses a size `wl_shm` cannot
//! take before anything that size exists: both run unchanged here, so
//! "allocates a lot" is never a finding the daemon would not refuse too.
//! One cap is the harness's own, for throughput and not a guard: a buffer
//! size `wl_shm` *would* take but of more than [`MAX_FUZZ_PIXELS`] is
//! skipped, since drawing a 16384×16384 buffer per run would spend the
//! fuzzer's time writing memory rather than finding paths. Sizes past
//! `wl_shm`'s limit are kept (they must be refused, and cheaply).
//!
//! # Input layout
//!
//! A [`HEADER`]-byte header, then the file's bytes:
//!
//! | Byte | Meaning |
//! |---|---|
//! | 0 | mode: [`Mode::ALL`] index, modulo 5 |
//! | 1 | filter: [`Filter::ALL`] index, modulo 4 |
//! | 2–4 | fill color, red, green, blue |
//! | 5 | orientation, modulo 9: 0 keeps the file's own (EXIF), 1–8 replace it |
//! | 6 | how many of the sizes below are drawn: 1 + (value modulo 3) |
//! | 7–18 | three buffer sizes, each width then height, `u16` little-endian ([`side`]) |
//!
//! Inputs shorter than the header do nothing.

use std::io::Cursor;

use scootbg_mem::shm::Geometry;

use crate::color::Color;
use crate::image::decode::decode;
use crate::image::orientation::Orientation;
use crate::image::render::{Look, render_each};
use crate::image::{DECODE_STACK, Filter, Mode};

#[cfg(test)]
mod tests;

/// Bytes before the file.
pub const HEADER: usize = 19;

/// The largest buffer drawn, in pixels (16 MiB of `XRGB8888`, so 1080p
/// and 1440p outputs are drawn): a throughput cap of the harness's, not a
/// guard (see the module docs).
pub const MAX_FUZZ_PIXELS: u64 = 1 << 22;

/// A buffer side from its two header bytes: the value itself (0 to 65532,
/// all but the top of a DRM mode's range), or, for the top three values,
/// a size only a compositor's `configure` could ask for: just past
/// `wl_shm`'s limit as a square, 2^20, and `u32::MAX`.
pub fn side(value: u16) -> u32 {
    match value {
        0xffff => u32::MAX,
        0xfffe => 1 << 20,
        // 23171² × 4 bytes is just over `i32::MAX`.
        0xfffd => 23_171,
        other => u32::from(other),
    }
}

/// Runs `data` (see the module docs for its layout) through decode and
/// [`render_each`], as the daemon's worker does for one job, on a thread
/// with the worker's stack ([`DECODE_STACK`]) rather than the caller's
/// (libFuzzer's main thread has 8 MiB): an input that overflows the
/// daemon's stack overflows here too. Panics only on a finding: a panic
/// from the path itself, or a drawn buffer that is not the size asked
/// for, or a size `wl_shm` cannot take that was drawn anyway.
pub fn whole_path(data: &[u8]) {
    let outcome = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("scootbg-decode".into())
            .stack_size(DECODE_STACK)
            .spawn_scoped(scope, || one_job(data))
            .expect("the harness cannot start its decoding thread")
            .join()
    });
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

/// [`whole_path`]'s body, on the decoding thread.
fn one_job(data: &[u8]) {
    let Some((header, file)) = data.split_first_chunk::<HEADER>() else {
        return;
    };
    let mode = Mode::ALL[usize::from(header[0]) % Mode::ALL.len()];
    let filter = Filter::ALL[usize::from(header[1]) % Filter::ALL.len()];
    let fill = Color {
        r: header[2],
        g: header[3],
        b: header[4],
    };
    let orientation = header[5] % 9;
    let count = 1 + usize::from(header[6]) % 3;

    // Distinct sizes, in order, as the worker passes them; at most three,
    // so no allocation.
    let mut sizes = [(0, 0); 3];
    let mut len = 0;
    for pair in header[7..].chunks_exact(4).take(count) {
        let dims = (
            side(u16::from_le_bytes([pair[0], pair[1]])),
            side(u16::from_le_bytes([pair[2], pair[3]])),
        );
        let drawable = Geometry::xrgb8888(dims.0, dims.1).is_ok();
        if drawable && u64::from(dims.0) * u64::from(dims.1) > MAX_FUZZ_PIXELS {
            continue;
        }
        if !sizes[..len].contains(&dims) {
            sizes[len] = dims;
            len += 1;
        }
    }

    let Ok(mut image) = decode(&mut Cursor::new(file), fill) else {
        return;
    };
    if orientation != 0 {
        image.orientation = Orientation::from_exif(u16::from(orientation));
    }
    let look = Look { mode, fill, filter };
    render_each(image, look, &sizes[..len], |dims, drawn| {
        let geometry = Geometry::xrgb8888(dims.0, dims.1);
        match (drawn, geometry) {
            (Ok(buffer), Ok(geometry)) => assert_eq!(
                buffer.geometry(),
                geometry,
                "a buffer drawn for {dims:?} is not that size"
            ),
            (Ok(_), Err(error)) => panic!("drew {dims:?}, which wl_shm refuses: {error}"),
            (Err(_), _) => {}
        }
    });
}
