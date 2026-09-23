//! What clipping windows to their own output costs: a frame of two
//! populated outputs, and the per-motion hit test.
//!
//! Both scenes have real client windows (the rounded bench's client) on two
//! side-by-side outputs, with the second output's first column scrolled
//! half off its left edge -- the overhang that used to draw onto, and take
//! clicks from, the first output (see
//! `docs/backlog/resolved/windows-bleed-across-outputs-done.md`). Run by hand:
//!
//! ```text
//! cargo test --release -p scoot --bin scoot output_clip -- --ignored --nocapture
//! ```
//!
//! Like the other suites that drive a real `State`, this needs a writable
//! `$XDG_RUNTIME_DIR`.

use std::hint::black_box;
use std::time::{Duration, Instant};

use scoot_core::{Action, Horizontal};
use smithay::utils::{Logical, Point};

use super::{CANVAS, RUNS, RoundedFixture, RoundedStep, WARMUP, render_frames, run_rounded_client};
use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::test_support::test_renderer;

/// Frames per timed run: every frame composites real client textures on
/// two outputs, so one frame costs milliseconds.
const FRAMES: u32 = 200;

/// Hit tests per timed run.
const PROBES: u32 = 200_000;

/// Two outputs, two half-width windows on the first and two two-thirds
/// windows on the second, with the second output's right column focused so
/// its left one hangs over the shared edge -- onto the first output's right
/// window.
fn two_output_scene() -> RoundedFixture {
    let mut fixture = RoundedFixture::headless(Appearance::default(), CANVAS);
    headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
        .expect("a second output");
    fixture.spawn(run_rounded_client);
    let half = f64::from(CANVAS / 2);
    // New windows open on the pointer's output.
    fixture.state.pointer_move(half, half);
    fixture.run(RoundedStep::Map);
    fixture.run(RoundedStep::Map);
    fixture.state.pointer_move(f64::from(CANVAS) + half, half);
    fixture.run(RoundedStep::Map);
    fixture.state.act(Action::SetColumnWidth(2));
    fixture.run(RoundedStep::Map);
    fixture.state.act(Action::SetColumnWidth(2));
    fixture.state.act(Action::FocusColumn(Horizontal::Right));
    let colors: [[u8; 4]; 4] = [
        [0x00, 0x00, 0xFF, 0xFF],
        [0x00, 0xFF, 0x00, 0xFF],
        [0xFF, 0x00, 0x00, 0xFF],
        [0xFF, 0xFF, 0x00, 0xFF],
    ];
    let arrangement = fixture.state.world.arrange();
    assert_eq!(arrangement.placements.len(), 4, "four mapped windows");
    let mut ids: Vec<_> = fixture.state.windows.keys().copied().collect();
    ids.sort();
    for (index, id) in ids.iter().enumerate() {
        let rect = arrangement.get(*id).expect("a placement").rect;
        fixture.run(RoundedStep::Attach {
            index,
            w: rect.w,
            h: rect.h,
            color: colors[index],
        });
    }
    let first = arrangement.get(ids[2]).expect("a placement").rect;
    assert!(
        first.x < CANVAS && first.right() > CANVAS,
        "the second output's left column must straddle the shared edge: {first:?}"
    );
    fixture
}

/// Per-frame cost of rendering both outputs, square and rounded.
#[test]
#[ignore = "prints per-frame render timings for a human; asserts nothing"]
fn output_clip_render_cost() {
    let renderer = test_renderer();
    let mut fixture = two_output_scene();
    for (label, radius) in [("square", 0), ("rounded 12", 12)] {
        fixture.state.appearance.corner_radius = radius;
        render_frames(&mut fixture, WARMUP);
        let mut best = Duration::MAX;
        for run in 1..=RUNS {
            let total = render_frames(&mut fixture, FRAMES);
            best = best.min(total);
            println!(
                "output-clip render [{renderer}], {label}, 2 outputs x 2 windows: {:?} per \
                 frame ({FRAMES} frames, 2x {CANVAS}x{CANVAS}, run {run}/{RUNS})",
                total / FRAMES
            );
        }
        println!(
            "output-clip render [{renderer}], {label}, 2 outputs x 2 windows: BEST {:?} per frame",
            best / FRAMES
        );
    }
}

/// Per-call cost of `surface_under`, the hit test every pointer motion runs,
/// at three kinds of point: inside a window on its own output, on the
/// second output's overhang where it crosses onto the first output (the
/// bleed pixels), and on bare background.
#[test]
#[ignore = "prints per-hit-test timings for a human; asserts nothing"]
fn output_clip_hit_test_cost() {
    let fixture = two_output_scene();
    let mid = f64::from(CANVAS / 2);
    let edge = f64::from(CANVAS);
    let points: [(&str, Point<f64, Logical>); 3] = [
        ("own window", (edge + edge * 0.8, mid).into()),
        ("bleed pixel", (edge - 20.0, mid).into()),
        ("background", (edge + 4.0, 4.0).into()),
    ];
    for (label, point) in points {
        for _ in 0..1000 {
            black_box(fixture.state.surface_under(black_box(point)));
        }
        let mut best = Duration::MAX;
        for run in 1..=RUNS {
            let started = Instant::now();
            for _ in 0..PROBES {
                black_box(fixture.state.surface_under(black_box(point)));
            }
            let total = started.elapsed();
            best = best.min(total);
            println!(
                "output-clip hit test, {label} at ({}, {}): {:?} per call ({PROBES} calls, run \
                 {run}/{RUNS})",
                point.x,
                point.y,
                total / PROBES
            );
        }
        println!(
            "output-clip hit test, {label}: BEST {:?} per call",
            best / PROBES
        );
    }
}
