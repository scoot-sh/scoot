//! What the layout's hot calls cost, printed for a human.
//!
//! `World::arrange` runs on every `apply()` (every event and action) and on
//! every rendered frame, so CLAUDE.md's "benchmark whenever a change touches
//! a hot path" lands here for any change to the layout. Asserts nothing -- a
//! wall-clock threshold is a flake, not a guarantee -- so it is `#[ignore]`d
//! like the compositor's `render_frame_cost`, and run by hand:
//!
//! ```text
//! cargo test --release -p scoot-core arrange_cost -- --ignored --nocapture
//! ```
//!
//! Scenes: [`WINDOWS`] windows over two outputs and three workspaces, with
//! some columns stacked, so every branch of `place_workspace` (on/off screen,
//! active/inactive workspace, stacked heights) is exercised. Each scene is
//! timed as `arrange()` alone, and as one focus step plus the `arrange()`
//! that follows it -- the shape `State::act` has.

use std::hint::black_box;
use std::time::{Duration, Instant};

use super::*;
use crate::{Action, Horizontal, Vertical};

const WINDOWS: u64 = 50;
const ROUNDS: u32 = 20_000;
const RUNS: u32 = 5;

fn scene() -> World {
    let mut world = World::new(config());
    world.handle_event(Event::OutputAdded {
        id: OutputId(1),
        area: SCREEN,
    });
    world.handle_event(Event::OutputAdded {
        id: OutputId(2),
        area: Rect::new(1000, 0, 1920, 1080),
    });
    for id in 1..=WINDOWS {
        open(&mut world, id);
        // Every fifth window joins the column to its left, so columns stack.
        if id % 5 == 0 {
            world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
        }
        // A new workspace every twenty windows, and the second output gets
        // the middle third.
        if id % 20 == 0 {
            world.handle_action(Action::FocusWorkspace(Vertical::Down));
        }
        if id == WINDOWS / 3 {
            world.handle_action(Action::FocusOutput(OutputId(2)));
        }
    }
    world
}

fn time(label: &str, mut body: impl FnMut()) {
    let mut runs: Vec<Duration> = (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..ROUNDS {
                body();
            }
            start.elapsed() / ROUNDS
        })
        .collect();
    runs.sort();
    println!(
        "{label}: median {:?}/call (min {:?}, max {:?}; {RUNS} runs x {ROUNDS})",
        runs[runs.len() / 2],
        runs[0],
        runs[runs.len() - 1],
    );
}

fn measure(label: &str, mut world: World) {
    time(&format!("{label} arrange"), || {
        black_box(world.arrange());
    });
    let mut right = true;
    time(&format!("{label} focus-step + arrange"), || {
        let dir = if right {
            Horizontal::Right
        } else {
            Horizontal::Left
        };
        right = !right;
        world.handle_action(Action::FocusColumn(dir));
        black_box(world.arrange());
    });
}

#[test]
#[ignore = "a timing printout, run by hand with --release --nocapture"]
fn arrange_cost() {
    measure("tiled", scene());
}
