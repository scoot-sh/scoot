//! What `note_pointer_activity` costs on the motion hot path, printed for
//! a human: `pointer_move` events with the feature off (one integer
//! compare) against on but idle (one clock read plus two stores, no
//! allocation, no timer churn).
//!
//! Asserts nothing -- a wall-clock threshold is a flake, not a guarantee --
//! so it is `#[ignore]`d like `popup_parent`'s bench, and run by hand:
//!
//! ```text
//! cargo test --release -p scoot --bin scoot pointer_move_cost_with_cursor_hide -- --ignored --nocapture
//! ```
//!
//! Per `CLAUDE.md`, any change touching input dispatch reports
//! before/after numbers from this bench.

use std::time::{Duration, Instant};

use super::*;

/// Motion events per timed run.
const EVENTS: u32 = 200_000;
/// Timed runs; the minimum is reported (noise only ever adds time).
const RUNS: u32 = 5;

/// `EVENTS` pointer moves over an empty session, unfocused throughout
/// (the common no-change path real motion takes at 500-1000 Hz).
fn moves(appearance: Appearance) -> Duration {
    let mut fixture: Harness<Step, Ack> = Harness::bare(appearance);
    // Warm up: arm the timer once, so the timed loop measures the steady
    // state rather than the one insert the first event performs.
    fixture.state.pointer_move(100.0, 100.0);
    let started = Instant::now();
    for i in 0..EVENTS {
        fixture.state.pointer_move(100.0 + f64::from(i % 7), 100.0);
    }
    started.elapsed()
}

#[test]
#[ignore]
fn pointer_move_cost_with_cursor_hide() {
    let mut best = Duration::MAX;
    for _ in 0..RUNS {
        best = best.min(moves(Appearance::default()));
    }
    eprintln!(
        "pointer_move, hide off: {best:?} for {EVENTS} events ({:.1} ns/event)",
        best.as_nanos() as f64 / f64::from(EVENTS),
    );
    let mut best = Duration::MAX;
    for _ in 0..RUNS {
        let appearance = Appearance {
            cursor_hide_after_ms: TIMEOUT.as_millis() as u64,
            ..Appearance::default()
        };
        best = best.min(moves(appearance));
    }
    eprintln!(
        "pointer_move, hide on:  {best:?} for {EVENTS} events ({:.1} ns/event)",
        best.as_nanos() as f64 / f64::from(EVENTS),
    );
}
