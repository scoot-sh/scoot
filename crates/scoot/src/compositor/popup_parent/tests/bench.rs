//! What a deep popup chain costs, printed for a human: creating it, and
//! drawing a frame with it open.
//!
//! Asserts nothing -- a wall-clock threshold is a flake, not a guarantee --
//! so it is `#[ignore]`d like `headless/bench.rs`, and run by hand:
//!
//! ```text
//! cargo test --release -p scoot --bin scoot popup_chain_cost -- --ignored --nocapture
//! ```
//!
//! `SCOOT_POPUP_DEPTH=N` sets the chain's length (default [`CAP`], the
//! deepest chain a client can have open). Past the cap the chain is
//! refused and this says so; against code without a cap it is how a chain
//! deep enough to overflow the compositor's stack is built on purpose.

use std::time::{Duration, Instant};

use super::*;

/// Frames per timed run.
const ROUNDS: u32 = 500;
/// Timed runs; the minimum is reported (noise only ever adds time).
const RUNS: u32 = 5;
/// Frames drawn before timing starts.
const WARMUP: u32 = 50;
/// Chains created per timed creation measurement, each in a fresh
/// compositor so the tree is the same size every time.
const CREATIONS: u32 = 5;

fn render_frames(fixture: &mut Fixture, rounds: u32) -> Duration {
    let started = Instant::now();
    for _ in 0..rounds {
        fixture.state.request_render();
        fixture.state.render();
    }
    started.elapsed()
}

/// The best time, over [`CREATIONS`] fresh compositors, for one client
/// batch of `len` nested popups to be sent, dispatched and answered.
///
/// Wall time of a whole round trip, so the client's side and the
/// harness's polling are in it too; `len = 1` is printed alongside as the
/// floor those contribute.
fn creation(len: usize) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..CREATIONS {
        let mut fixture = Fixture::with_window();
        let started = Instant::now();
        fixture.batch(vec![Op::Chain {
            parent: Parent::Window(0),
            len,
        }]);
        best = best.min(started.elapsed());
    }
    best
}

#[test]
#[ignore = "prints popup-chain timings for a human; asserts nothing"]
fn popup_chain_cost() {
    let depth = std::env::var("SCOOT_POPUP_DEPTH")
        .ok()
        .and_then(|depth| depth.parse().ok())
        .unwrap_or(CAP);
    println!(
        "popup chain: depth {depth}, {} build",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );

    if depth > CAP {
        // Built and drawn once, not timed: against code with the cap, the
        // client is refused; against code without one, this is the crash.
        let mut fixture = Fixture::with_window();
        let started = Instant::now();
        fixture.send_step(
            0,
            Step::Batch(vec![Op::Chain {
                parent: Parent::Window(0),
                len: depth,
            }]),
        );
        let survived =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fixture.wait_for_ack(0)))
                .is_ok();
        println!(
            "popup chain: depth {depth} {} after {:?}",
            if survived {
                "ADMITTED"
            } else {
                "REFUSED (client disconnected)"
            },
            started.elapsed()
        );
        let started = Instant::now();
        fixture.state.request_render();
        fixture.state.render();
        println!("popup chain: one frame after it: {:?}", started.elapsed());
        return;
    }

    let floor = creation(1);
    let chain = creation(depth);
    println!("popup chain: create 1 popup: BEST {floor:?} (round trip floor)");
    println!("popup chain: create {depth} nested popups in one batch: BEST {chain:?}");

    for (label, len) in [("window, no popups", 0), ("window + chain", depth)] {
        let mut fixture = Fixture::with_window();
        if len > 0 {
            fixture.batch(vec![Op::Chain {
                parent: Parent::Window(0),
                len,
            }]);
            fixture.map(0..len, Some(len - 1));
        }
        render_frames(&mut fixture, WARMUP);
        let mut best = Duration::MAX;
        for _ in 0..RUNS {
            best = best.min(render_frames(&mut fixture, ROUNDS));
        }
        println!(
            "popup chain: frame, {label} ({len}): BEST {:?} per frame ({ROUNDS} frames, \
             {CANVAS}x{CANVAS})",
            best / ROUNDS
        );
    }
}
