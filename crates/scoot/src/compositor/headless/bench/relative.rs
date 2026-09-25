//! What one *relative* pointer motion costs with one output and with two:
//! `State::pointer_move_relative`, `--tty` libinput's only motion path, whose
//! clamp walks every output's geometry when there is more than one
//! (`State::clamp_to_output_union`). Two outputs had no production producer
//! until `--tty` drove every connector (milestone 19, phase E); this is the
//! number that change owes. Run by hand:
//!
//! ```text
//! cargo test -p scoot --bin scoot relative_motion -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Headless outputs with no render target, so the motion path is all that is
//! timed: the pointer oscillates a pixel either side of a point well inside
//! the first output, never crossing a seam or reaching an edge.

use std::time::{Duration, Instant};

use crate::compositor::decorations::Appearance;
use crate::compositor::headless::add_output_without_backend;
use crate::compositor::test_support::Harness;

const CANVAS: i32 = 800;
const MOTIONS: u32 = 200_000;
const RUNS: usize = 5;

fn time(outputs: i32) -> Vec<Duration> {
    let mut harness: Harness<(), ()> = Harness::bare(Appearance::default());
    for index in 0..outputs {
        let id = add_output_without_backend(
            &mut harness.state,
            &format!("bench-{index}"),
            CANVAS,
            CANVAS,
        );
        if index > 0 {
            // Side by side, like every multi-output session.
            if let Some(output) = harness.state.outputs.get(id).cloned() {
                harness.state.space.map_output(&output, (CANVAS * index, 0));
            }
        }
    }
    harness
        .state
        .pointer_move(f64::from(CANVAS) / 2.0, f64::from(CANVAS) / 2.0);
    let step = |harness: &mut Harness<(), ()>, i: u32| {
        let d = if i % 2 == 0 { 1.0 } else { -1.0 };
        harness.state.pointer_move_relative(d, 0.0, d, 0.0);
    };
    for i in 0..MOTIONS / 10 {
        step(&mut harness, i);
    }
    (0..RUNS)
        .map(|_| {
            let started = Instant::now();
            for i in 0..MOTIONS {
                step(&mut harness, i);
            }
            started.elapsed()
        })
        .collect()
}

fn report(outputs: i32, mut runs: Vec<Duration>) {
    runs.sort();
    let per = |d: Duration| d / MOTIONS;
    println!(
        "relative motion, {outputs} output(s): median {:?} per motion (min {:?}, max {:?}; {RUNS} runs x {MOTIONS})",
        per(runs[RUNS / 2]),
        per(runs[0]),
        per(runs[RUNS - 1]),
    );
}

#[test]
#[ignore = "prints per-motion timings for a human; asserts nothing"]
fn relative_motion_cost() {
    report(1, time(1));
    report(2, time(2));
}
