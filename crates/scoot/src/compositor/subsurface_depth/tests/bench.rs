//! What a deep subsurface tree costs, printed for a human: creating it, and
//! drawing a frame with it mapped -- alone, and below a popup chain at the
//! popup cap.
//!
//! Asserts nothing -- a wall-clock threshold is a flake, not a guarantee --
//! so it is `#[ignore]`d like `popup_parent`'s bench, and run by hand:
//!
//! ```text
//! cargo test --release -p scoot --bin scoot subsurface_chain_cost -- --ignored --nocapture
//! ```
//!
//! `SCOOT_SUBSURFACE_DEPTH=N` sets the chain's depth (default [`CAP`], the
//! deepest a client can have). Past the cap the chain is refused and this
//! says so; against code without a cap it is how a tree deep enough to
//! overflow the compositor's stack is built on purpose.

use std::time::{Duration, Instant};

use super::*;
use crate::compositor::popup_parent::tests::{CAP as POPUP_CAP, Parent, Step};

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
/// batch of `len` nested, drawn subsurfaces to be sent, dispatched and
/// answered -- a whole round trip, so `len = 1` is printed alongside as
/// the floor the client and the harness contribute.
fn creation(len: usize) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..CREATIONS {
        let mut fixture = Fixture::with_window();
        let started = Instant::now();
        fixture.batch(vec![chain(Node::Window(0), len)]);
        best = best.min(started.elapsed());
    }
    best
}

/// The best per-frame time over [`RUNS`] runs of [`ROUNDS`] frames.
fn frame(fixture: &mut Fixture) -> Duration {
    render_frames(fixture, WARMUP);
    let mut best = Duration::MAX;
    for _ in 0..RUNS {
        best = best.min(render_frames(fixture, ROUNDS));
    }
    best / ROUNDS
}

#[test]
#[ignore = "prints subsurface-tree timings for a human; asserts nothing"]
fn subsurface_chain_cost() {
    let depth = std::env::var("SCOOT_SUBSURFACE_DEPTH")
        .ok()
        .and_then(|depth| depth.parse().ok())
        .unwrap_or(CAP);
    println!(
        "subsurface chain: depth {depth}, {} build",
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
        fixture.send_step(0, Step::Batch(vec![chain(Node::Window(0), depth)]));
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fixture.wait_for_ack(0)));
        let outcome = match outcome {
            Ok(_) => "ADMITTED".to_owned(),
            Err(panic) => panic
                .downcast_ref::<String>()
                .map_or("NOT ANSWERED", String::as_str)
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned(),
        };
        println!(
            "subsurface chain: depth {depth} after {:?}: {outcome}",
            started.elapsed()
        );
        let started = Instant::now();
        fixture.state.request_render();
        fixture.state.render();
        println!(
            "subsurface chain: one frame after it: {:?}",
            started.elapsed()
        );
        return;
    }

    let floor = creation(1);
    let built = creation(depth);
    println!("subsurface chain: create 1 subsurface: BEST {floor:?} (round trip floor)");
    println!("subsurface chain: create {depth} nested subsurfaces in one batch: BEST {built:?}");

    let mut fixture = Fixture::with_window();
    println!(
        "subsurface chain: frame, window alone: BEST {:?} per frame ({ROUNDS} frames, \
         {CANVAS}x{CANVAS})",
        frame(&mut fixture)
    );

    let mut fixture = Fixture::with_window();
    fixture.batch(vec![chain(Node::Window(0), depth)]);
    println!(
        "subsurface chain: frame, window + {depth} nested subsurfaces: BEST {:?} per frame",
        frame(&mut fixture)
    );

    let mut fixture = Fixture::with_window();
    fixture.batch(vec![Op::Chain {
        parent: Parent::Window(0),
        len: POPUP_CAP,
    }]);
    fixture.map(0..POPUP_CAP, None);
    println!(
        "subsurface chain: frame, window + {POPUP_CAP} nested popups: BEST {:?} per frame",
        frame(&mut fixture)
    );
    fixture.batch(vec![chain(Node::Popup(POPUP_CAP - 1), depth)]);
    println!(
        "subsurface chain: frame, window + {POPUP_CAP} nested popups + {depth} nested \
         subsurfaces below the deepest: BEST {:?} per frame",
        frame(&mut fixture)
    );
}
