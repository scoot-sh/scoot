//! What one [`State::apply`](crate::compositor::State) costs with real
//! client windows mapped: the path every action, IPC request, map, unmap and
//! output change ends in. Not per frame, but on the IPC/action path an agent
//! can drive at its own rate, and the per-window walk in it grows with the
//! window count -- `output_scale.rs`'s `refresh_window_scales` added one
//! `Cell` compare per window per call, and this is where that shows or not.
//!
//! Nothing changes between calls, so every configure is a no-op and every
//! window stays on its output: the steady-state cost, the common case. Run
//! by hand:
//!
//! ```text
//! cargo test --release -p scoot --bin scoot apply_cost -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Like the other suites that drive a real `State`, this needs a writable
//! `$XDG_RUNTIME_DIR`.

use std::time::{Duration, Instant};

use super::{RUNS, RoundedFixture, rounded_scene};

/// `apply()` calls per timed run.
const CALLS: u32 = 20_000;

fn time_applies(fixture: &mut RoundedFixture, calls: u32) -> Duration {
    let started = Instant::now();
    for _ in 0..calls {
        fixture.state.apply();
    }
    started.elapsed()
}

#[test]
#[ignore = "prints per-call timings for a human; asserts nothing"]
fn apply_cost() {
    for windows in [1, 8] {
        let mut fixture = rounded_scene(windows, 1);
        time_applies(&mut fixture, CALLS / 10);
        let mut runs: Vec<Duration> = (0..RUNS)
            .map(|_| time_applies(&mut fixture, CALLS))
            .collect();
        runs.sort();
        let per = |d: Duration| d / CALLS;
        println!(
            "apply(), {windows} client windows: median {:?} per call (min {:?}, max {:?}; {} runs x {CALLS})",
            per(runs[runs.len() / 2]),
            per(runs[0]),
            per(runs[runs.len() - 1]),
            runs.len(),
        );
    }
    // The same eight windows with a second output at another scale, so the
    // per-window walk actually runs (`State::mixed_scales`): what an
    // `[[outputs]]` session pays per call, windows all staying put.
    let mut fixture = rounded_scene(8, 1);
    fixture.state.output_entries = crate::compositor::output_config::OutputEntries::from_toml(
        "[[outputs]]\nname = \"headless-2\"\nscale = 2\n",
    );
    crate::compositor::headless::add_output(
        &mut fixture.state,
        "headless-2",
        super::CANVAS,
        super::CANVAS,
    )
    .expect("a second output");
    assert!(
        fixture.state.mixed_scales,
        "the walk must be on for this scene"
    );
    time_applies(&mut fixture, CALLS / 10);
    let mut runs: Vec<Duration> = (0..RUNS)
        .map(|_| time_applies(&mut fixture, CALLS))
        .collect();
    runs.sort();
    let per = |d: Duration| d / CALLS;
    println!(
        "apply(), 8 client windows, outputs at two scales: median {:?} per call (min {:?}, max {:?}; {} runs x {CALLS})",
        per(runs[runs.len() / 2]),
        per(runs[0]),
        per(runs[runs.len() - 1]),
        runs.len(),
    );
}
