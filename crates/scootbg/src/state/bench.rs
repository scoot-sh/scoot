//! What saving costs: an `#[ignore]`d test, run by hand in a release
//! build:
//!
//! ```sh
//! SCOOTBG_BENCH_DIR=~/.local/state/scootbg-bench cargo test --release \
//!     -p scootbg -- --ignored --nocapture --test-threads 1 state::bench
//! ```
//!
//! On the loop: building the file's text ([`Saved::text`]) and handing it
//! over ([`Saver::save`], which starts the writing thread). On that
//! thread: the atomic write itself ([`write_atomic`]: temporary file,
//! `fsync`, `rename`, `fsync` of the directory), on the file system
//! `SCOOTBG_BENCH_DIR` is on (the system temporary directory by default).
//! 200 rounds each; the record these numbers went into is
//! docs/scootbg/backlog/resolved/restore-state-done.md.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::saver::{Saver, write_atomic};
use super::{Profile, Saved};
use crate::color::Color;
use crate::image::render::Look;
use crate::image::{Filter, Mode};
use crate::wallpaper::{Image, Wallpaper};

const ROUNDS: usize = 200;

fn summary(what: &str, mut times: Vec<Duration>) {
    times.sort();
    let at = |q: f64| times[((times.len() - 1) as f64 * q) as usize].as_secs_f64() * 1e6;
    println!(
        "{what}: median {:.1} us, p90 {:.1} us, p99 {:.1} us, max {:.1} us ({} rounds)",
        at(0.5),
        at(0.9),
        at(0.99),
        at(1.0),
        times.len()
    );
}

#[test]
#[ignore = "a benchmark: run by hand, see the module docs"]
fn saving() {
    let dir = std::env::var_os("SCOOTBG_BENCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("sbg-bench-{}", std::process::id())));
    let file = dir.join("bench");
    let image = |path: &str| {
        Some(Wallpaper::Image(Arc::new(Image {
            path: path.to_owned(),
            look: Look {
                mode: Mode::Fill,
                fill: Color { r: 0, g: 0, b: 0 },
                filter: Filter::Lanczos3,
            },
            serial: 1,
        })))
    };
    // A typical file: an image for every output, and three by name.
    let mut saved = Saved::new(Profile::default(), None, Some("f".repeat(64)));
    saved.choices.set(
        None,
        image("/home/me/Pictures/Wallpapers/a hill at dusk.jpg"),
        1,
    );
    saved
        .choices
        .set(Some("DP-1"), image("/home/me/Pictures/b.png"), 2);
    saved.choices.set(
        Some("HDMI-A-1"),
        Some(Wallpaper::Color(Color { r: 1, g: 2, b: 3 })),
        3,
    );
    saved.choices.set(Some("eDP-1"), None, 4);
    let text = saved.text();
    println!("file: {} bytes on {}", text.len(), dir.display());

    let mut build = Vec::with_capacity(ROUNDS);
    for _ in 0..ROUNDS {
        let started = Instant::now();
        let text = std::hint::black_box(saved.text());
        build.push(started.elapsed());
        drop(text);
    }
    summary("loop: build the text", build);

    let saver = Saver::new(file.clone());
    let mut hand = Vec::with_capacity(ROUNDS);
    for _ in 0..ROUNDS {
        let text = saved.text();
        let started = Instant::now();
        saver.save(text);
        hand.push(started.elapsed());
        assert!(saver.flush(Duration::from_secs(30)));
    }
    summary("loop: hand it over (starts a thread)", hand);

    let mut write = Vec::with_capacity(ROUNDS);
    for _ in 0..ROUNDS {
        let started = Instant::now();
        write_atomic(&file, text.as_bytes()).unwrap();
        write.push(started.elapsed());
    }
    summary("thread: write it atomically", write);
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir(&dir);
}
