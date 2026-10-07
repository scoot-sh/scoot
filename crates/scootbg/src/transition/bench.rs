//! Transition frame costs, per kind and size: an `#[ignore]`d test, run
//! by hand in a release build:
//!
//! ```sh
//! cargo test --release -p scootbg -- \
//!     --ignored --nocapture --test-threads 1 transition::bench
//! ```
//!
//! Each kind blends 30 frames at 1920×1080 and 3840×2160 between two
//! gradients (so every pixel moves), reporting the median milliseconds
//! per frame and the damage rects of a mid frame. The record these
//! numbers went into is `docs/scootbg/README.md` (the transition rows of
//! "Measured so far").

use std::time::Instant;

use super::{Easing, Kind, Spec, Sweep, blend_row, damage};

fn spec(kind: Kind) -> Spec {
    Spec {
        kind,
        duration_ms: 500,
        easing: Easing::Linear,
        angle_deg: 30.0,
        pos: (0.3, 0.7),
    }
}

/// Old and new buffers of `dims`, distinct gradients, plus scratch.
fn buffers(dims: (u32, u32)) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let len = dims.0 as usize * dims.1 as usize * 4;
    let mut old = vec![0u8; len];
    let mut new = vec![0u8; len];
    for y in 0..dims.1 {
        for x in 0..dims.0 {
            let o = (y as usize * dims.0 as usize + x as usize) * 4;
            old[o] = (x % 256) as u8;
            old[o + 1] = (y % 256) as u8;
            old[o + 2] = ((x + y) % 256) as u8;
            old[o + 3] = 0xff;
            new[o] = (255 - x % 256) as u8;
            new[o + 1] = (255 - y % 256) as u8;
            new[o + 2] = ((x * y) % 256) as u8;
            new[o + 3] = 0xff;
        }
    }
    (old, new, vec![0u8; len])
}

fn ms_per_frame(kind: Kind, dims: (u32, u32)) -> f64 {
    let (old, new, mut out) = buffers(dims);
    let row_len = dims.0 as usize * 4;
    let mut times: Vec<f64> = Vec::with_capacity(30);
    // eased progresses across the transition, as the driver renders them.
    for frame in 0..30 {
        let eased = (f64::from(frame) + 1.0) / 31.0;
        let sweep = Sweep::new(&spec(kind), eased, dims);
        let start = Instant::now();
        for y in 0..dims.1 {
            let o = y as usize * row_len;
            blend_row(
                &sweep,
                y,
                &old[o..o + row_len],
                &new[o..o + row_len],
                &mut out[o..o + row_len],
            );
        }
        std::hint::black_box(&out);
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

#[test]
#[ignore = "a benchmark: run by hand in a release build, see the module docs"]
fn frames() {
    for dims in [(1920, 1080), (3840, 2160)] {
        for kind in [Kind::Fade, Kind::Wipe, Kind::Grow] {
            let ms = ms_per_frame(kind, dims);
            let rects = damage(kind, 0.4, 0.5, dims, 30.0, (0.3, 0.7));
            println!(
                "{kind:?} {}x{}: {ms:.2} ms/frame, {} damage rects",
                dims.0, dims.1, rects.n,
            );
        }
    }
}
