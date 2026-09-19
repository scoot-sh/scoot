//! What one frame of [`State::render`](crate::compositor::State::render)
//! costs, printed for a human.
//!
//! CLAUDE.md asks for real before/after numbers whenever a change lands on a
//! hot path, and this is the hottest one. Asserts nothing -- a wall-clock
//! threshold in CI is a flake, not a guarantee -- so it is `#[ignore]`d like
//! `cursor/shapes/tests.rs`'s shape dump and `dmabuf/tests.rs`'s commit-sync
//! timing, and run by hand:
//!
//! ```text
//! cargo test --release -p scoot --bin scoot render_frame_cost -- --ignored --nocapture
//! ```
//!
//! ## What the scenes measure, and what they deliberately do not
//!
//! Two scenes, both driving the real `PixmanRenderer` into a real
//! [`CANVAS`]-square framebuffer:
//!
//! - **empty desktop** -- no windows, no layer surfaces, no cursor: the
//!   damage tracker's clear plus `render()`'s own fixed per-frame work
//!   (output geometry, the layer-map lock, the frame-callback and cleanup
//!   pass). This is the *most sensitive* scene for a change to the frame
//!   loop's structure, because that fixed cost is the whole measurement
//!   rather than a sliver of it.
//! - **`RING_WINDOWS` windows** -- the same, plus a real `World::arrange`
//!   and the four focus-ring elements per placement that
//!   `Decorations::elements` builds from it, composited by pixman.
//!
//! The windows exist in [`scoot_core`] only: they are pushed straight into
//! the core with `WindowOpened` rather than mapped by a client, so the ring
//! is drawn for each of them and `Space::render_elements_for_region` still
//! finds nothing to draw. That is deliberate -- a client surface's texture
//! import is *pixman's* cost, essentially constant against any change to how
//! the frame is assembled, and including it would bury exactly the fixed
//! overhead these numbers exist to watch. A change that claims to be free
//! has to be free here.
//!
//! Like the other suites that drive a real `State`, this needs a writable
//! `$XDG_RUNTIME_DIR`.

use std::time::{Duration, Instant};

use scoot_core::{Event as CoreEvent, WindowId, WindowInfo};

use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::Harness;

/// The framebuffer each scene renders into. 800 square, matching the
/// headless-render benchmark recorded in
/// `docs/backlog/resolved/present-skip-eats-frame-damage-done.md` so the two
/// numbers are comparable.
const CANVAS: i32 = 800;

/// Frames per timed run.
const ROUNDS: u32 = 500;

/// Timed runs per scene.
const RUNS: u32 = 5;

/// Frames rendered before timing starts, so the first-frame costs (the
/// damage tracker's full-output redraw, the pixman image faulting in) do not
/// land in the average.
const WARMUP: u32 = 50;

/// How many windows the second scene arranges.
const RING_WINDOWS: u64 = 8;

/// No client ever connects here, so the step/ack vocabulary is empty.
type Fixture = Harness<(), ()>;

/// A live compositor with a real headless backend and `windows` windows in
/// the core.
fn scene(windows: u64) -> Fixture {
    let mut fixture = Harness::headless(Appearance::default(), CANVAS);
    for index in 0..windows {
        fixture.state.world.handle_event(CoreEvent::WindowOpened {
            id: WindowId(index + 1),
            info: WindowInfo {
                app_id: "bench".to_string(),
                title: "bench".to_string(),
                hints: Default::default(),
            },
            output: None,
            focus: true,
        });
    }
    fixture
}

/// Renders `rounds` frames back to back and hands back the total.
///
/// `request_render` before each one because `render()` returns immediately
/// on a clean screen -- which is the behaviour under measurement's own
/// fast path, not something being worked around: what is being timed is the
/// cost of a frame that really draws.
fn render_frames(fixture: &mut Fixture, rounds: u32) -> Duration {
    let started = Instant::now();
    for _ in 0..rounds {
        fixture.state.request_render();
        fixture.state.render();
    }
    started.elapsed()
}

#[test]
#[ignore = "prints per-frame render timings for a human; asserts nothing"]
fn render_frame_cost() {
    for (label, windows) in [("empty desktop", 0), ("8 windows", RING_WINDOWS)] {
        let mut fixture = scene(windows);
        render_frames(&mut fixture, WARMUP);
        // Five runs rather than one: this is a VM sharing a host's cores, so
        // a single number says nothing about whether a difference between
        // two builds is real. Compare the *minima* -- noise only ever adds
        // time, so the fastest run of each build is the one least polluted
        // by whatever else the host was doing.
        let mut best = Duration::MAX;
        for run in 1..=RUNS {
            let total = render_frames(&mut fixture, ROUNDS);
            best = best.min(total);
            println!(
                "render, {label}: {total:?} total, {:?} per frame ({ROUNDS} frames, \
                 {CANVAS}x{CANVAS}, run {run}/{RUNS})",
                total / ROUNDS
            );
        }
        println!(
            "render, {label}: BEST {:?} per frame ({ROUNDS} frames, {CANVAS}x{CANVAS})",
            best / ROUNDS
        );
    }
}
