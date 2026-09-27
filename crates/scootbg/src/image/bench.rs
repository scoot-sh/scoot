//! The pipeline, stage by stage, on real files: an `#[ignore]`d test, run
//! by hand in a release build:
//!
//! ```sh
//! SCOOTBG_BENCH="a.jpg b.png" cargo test --release -p scootbg -- \
//!     --ignored --nocapture --test-threads 1 image::bench
//! ```
//!
//! Each file is drawn `fill` at 3840×2160 (`SCOOTBG_BENCH_SIZE=WxH` to
//! change it) five times. The stages are `render`'s own, called one by one
//! with the same functions, then `render` whole; peak RSS (`VmHWM`, reset
//! through `/proc/self/clear_refs` before each run) and the heap left
//! afterwards (`RssAnon`) are read from `/proc/self/status`. The record
//! these numbers went into is
//! docs/scootbg/backlog/resolved/images-decode-and-fit-done.md.

use std::path::Path;
use std::time::Instant;

use scootbg_mem::ShmBuffer;

use super::decode::decode_file;
use super::fit::{self, Rect};
use super::pack::{Stored, Target};
use super::render::{Look, Source, crop_in_place, render};
use super::scale::scale;
use super::{Filter, Mode};
use crate::color::Color;

fn status(key: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix(key))
        .and_then(|v| v.trim().trim_end_matches(" kB").parse().ok())
        .unwrap_or(0)
}

fn reset_peak() {
    std::fs::write("/proc/self/clear_refs", "5").unwrap();
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

#[test]
#[ignore = "a benchmark: run by hand in a release build, see the module docs"]
fn pipeline() {
    let files = std::env::var("SCOOTBG_BENCH").expect("SCOOTBG_BENCH=\"FILE...\"");
    let dims = std::env::var("SCOOTBG_BENCH_SIZE")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or((3840, 2160));
    let look = Look {
        mode: Mode::Fill,
        fill: Color { r: 0, g: 0, b: 0 },
        filter: Filter::Lanczos3,
    };
    for file in files.split_whitespace() {
        for run in 0..5 {
            let base = status("RssAnon:");
            reset_peak();
            let start = Instant::now();
            let t = Instant::now();
            let mut decoded = decode_file(Path::new(file), look.fill).unwrap();
            let decode = ms(t);
            let (w, h, o) = (decoded.width, decoded.height, decoded.orientation);
            let layout = fit::layout(look.mode, o.displayed(w, h), dims).unwrap();
            let crop = o.rect_to_stored(layout.crop, w, h);
            let to = o.stored(layout.scaled.0, layout.scaled.1);
            let t = Instant::now();
            if crop != Rect::whole(w, h) {
                crop_in_place(&mut decoded.rgb, w, h, crop).unwrap();
            }
            let cropping = ms(t);
            let t = Instant::now();
            let scaled = scale(&decoded.rgb, (crop.width, crop.height), to, look.filter).unwrap();
            let scaling = ms(t);
            drop(decoded);
            let t = Instant::now();
            let mut buffer = ShmBuffer::new(dims.0, dims.1).unwrap();
            let alloc = ms(t);
            let t = Instant::now();
            let mut target = Target::new(buffer.pixels_mut(), dims.0, dims.1).unwrap();
            let stored = Stored {
                rgb: &scaled,
                width: to.0,
                height: to.1,
                orientation: o,
            };
            target
                .draw(
                    stored,
                    Rect::whole(layout.scaled.0, layout.scaled.1),
                    layout.at,
                )
                .unwrap();
            let pack = ms(t);
            drop(scaled);
            let total = ms(start);
            let peak = status("VmHWM:");
            drop(buffer);
            // `render` whole, as the worker calls it.
            reset_peak();
            let t = Instant::now();
            let decoded = decode_file(Path::new(file), look.fill).unwrap();
            let buffer = render(Source::Owned(decoded), look, dims).unwrap();
            let whole = ms(t);
            let whole_peak = status("VmHWM:");
            drop(buffer);
            let after = status("RssAnon:");
            println!(
                "{file} run {run} orientation {}: decode {decode:.1} crop {cropping:.1} \
                 scale {scaling:.1} shm {alloc:.1} pack {pack:.1} = {total:.1} ms, peak \
                 {peak} kB; render() {whole:.1} ms, peak {whole_peak} kB; heap {base} -> \
                 {after} kB",
                o.value()
            );
        }
    }
}
