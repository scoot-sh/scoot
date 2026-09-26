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

/// How many side-by-side popups the sibling bench builds, unless
/// `SCOOT_POPUP_SIBLINGS` says otherwise. In the ticket's shapes: ~2000 is
/// where the stall is a second-ish in release on the dev VM.
fn siblings() -> usize {
    std::env::var("SCOOT_POPUP_SIBLINGS")
        .ok()
        .and_then(|siblings| siblings.parse().ok())
        .unwrap_or(2000)
}

/// One batch of `n` side-by-side popups of the window, all sent in one
/// flush.
fn sibling_batch(n: usize) -> Vec<Op> {
    std::iter::repeat_n(Op::Popup(Parent::Window(0)), n).collect()
}

/// The best time, over [`CREATIONS`] fresh compositors, to send, dispatch
/// and answer one client batch of `n` side-by-side popups.
///
/// Wall time of a whole round trip, so the client's side and the harness's
/// polling are in it too. Hands back whether the batch was admitted: past a
/// per-client count cap the client is refused instead, and the time is the
/// time to refuse it.
fn sibling_creation(n: usize) -> (Duration, bool) {
    let mut best = Duration::MAX;
    let mut admitted = false;
    for _ in 0..CREATIONS {
        let mut fixture = Fixture::with_window();
        let started = Instant::now();
        let outcome = fixture.run_or_disconnect(Step::Batch(sibling_batch(n)));
        best = best.min(started.elapsed());
        admitted = outcome.is_ok();
    }
    (best, admitted)
}

/// What thousands of popups open side by side cost, printed for a human:
/// building them (one flush: tracking plus every first commit's configure),
/// drawing a frame with them mapped, and tearing them all down again.
///
/// The ticket (`docs/backlog/core/popup-count-quadratic.md`) measured this
/// roughly quadratic: 1954 popups for 0.73 s, 5104 for 5.4 s (release, dev
/// VM). Asserts nothing -- a wall-clock threshold is a flake, not a
/// guarantee -- so it is `#[ignore]`d like [`popup_chain_cost`], and run by
/// hand:
///
/// ```text
/// SCOOT_POPUP_SIBLINGS=2000 cargo test --release -p scoot --bin scoot popup_sibling_cost -- --ignored --nocapture
/// ```
///
/// The build is split in two so the profile names the hot spot: `Uncommitted`
/// popups are tracked but never committed (no initial configure, no
/// `find_popup`), and the `Commit` batch afterwards is the commits alone.
#[test]
#[ignore = "prints popup-sibling timings for a human; asserts nothing"]
fn popup_sibling_cost() {
    let n = siblings();
    println!(
        "popup siblings: {n} side by side, {} build",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );

    let uncommitted = {
        let mut best = Duration::MAX;
        let mut admitted = true;
        for _ in 0..CREATIONS {
            let mut fixture = Fixture::with_window();
            let ops: Vec<Op> = std::iter::repeat_n(Op::Uncommitted(Parent::Window(0)), n).collect();
            let started = Instant::now();
            admitted = fixture
                .run_or_disconnect(Step::Batch(ops))
                .map(|Ack::Done| ())
                .is_ok()
                && admitted;
            best = best.min(started.elapsed());
        }
        (best, admitted)
    };
    println!("popup siblings: track {n} uncommitted in one batch: BEST {uncommitted:?}");

    let commits = {
        let mut best = Duration::MAX;
        let mut admitted = true;
        for _ in 0..CREATIONS {
            let mut fixture = Fixture::with_window();
            let ops: Vec<Op> = std::iter::repeat_n(Op::Uncommitted(Parent::Window(0)), n).collect();
            admitted = fixture
                .run_or_disconnect(Step::Batch(ops))
                .map(|Ack::Done| ())
                .is_ok()
                && admitted;
            let ops: Vec<Op> = (0..n).map(Op::Commit).collect();
            let started = Instant::now();
            admitted = fixture
                .run_or_disconnect(Step::Batch(ops))
                .map(|Ack::Done| ())
                .is_ok()
                && admitted;
            best = best.min(started.elapsed());
        }
        (best, admitted)
    };
    println!("popup siblings: commit {n} tracked popups in one batch: BEST {commits:?}");

    let build = sibling_creation(n);
    println!("popup siblings: create {n} committed popups in one batch: BEST {build:?}");

    let mut fixture = Fixture::with_window();
    let started = Instant::now();
    let admitted = fixture
        .run_or_disconnect(Step::Batch(sibling_batch(n)))
        .is_ok();
    let built = started.elapsed();
    if !admitted {
        println!("popup siblings: create {n} committed popups: REFUSED after {built:?}");
        return;
    }
    println!("popup siblings: admitted {n} committed popups after {built:?} (one sample)");
    {
        let started = Instant::now();
        fixture.map(0..n, None);
        println!(
            "popup siblings: ack and draw {n} popups: {:?}",
            started.elapsed()
        );

        render_frames(&mut fixture, WARMUP);
        let mut best = Duration::MAX;
        for _ in 0..RUNS {
            best = best.min(render_frames(&mut fixture, ROUNDS));
        }
        println!(
            "popup siblings: frame, window + {n} mapped popups: BEST {:?} per frame ({ROUNDS} frames, \
         {CANVAS}x{CANVAS})",
            best / ROUNDS
        );

        let started = Instant::now();
        fixture.batch((0..n).map(Op::Destroy).collect());
        println!(
            "popup siblings: destroy {n} popups in one batch: {:?}",
            started.elapsed()
        );
    }
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
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fixture.wait_for_ack(0)));
        let outcome = match outcome {
            Ok(_) => "ADMITTED".to_owned(),
            // The harness's own diagnosis: the client's protocol error, or
            // "timed out" if a single dispatch outlasted its patience.
            Err(panic) => panic
                .downcast_ref::<String>()
                .map_or("NOT ANSWERED", String::as_str)
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned(),
        };
        println!(
            "popup chain: depth {depth} after {:?}: {outcome}",
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
