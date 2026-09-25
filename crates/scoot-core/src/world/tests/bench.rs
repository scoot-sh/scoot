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
//! active/inactive workspace, stacked heights) is exercised -- tiled, with the
//! focused window fullscreen, and with three floating windows added. Each scene is
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

fn time(label: &str, body: impl FnMut()) {
    time_rounds(label, ROUNDS, body);
}

fn time_rounds(label: &str, rounds: u32, mut body: impl FnMut()) {
    let mut runs: Vec<Duration> = (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..rounds {
                body();
            }
            start.elapsed() / rounds
        })
        .collect();
    runs.sort();
    println!(
        "{label}: median {:?}/call (min {:?}, max {:?}; {RUNS} runs x {rounds})",
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
    // The same scene with the focused window fullscreen: the focus step then
    // alternates between covering and not, which is the path that pays for
    // the fullscreen width lookup per column.
    let mut fullscreen = scene();
    fullscreen.handle_action(Action::ToggleFullscreen);
    measure("fullscreen", fullscreen);
    // The same scene with three floating windows on the focused workspace
    // (drawn, so placed visible), focus back on the strip: the focus step
    // then pays for placing the floating layer on every arrangement.
    let mut floating = scene();
    for id in WINDOWS + 1..=WINDOWS + 3 {
        open(&mut floating, id);
        floating.handle_event(Event::FloatingRequested {
            id: WindowId(id),
            floating: true,
            size: None,
        });
        floating.handle_event(Event::FrameObserved {
            id: WindowId(id),
            requested: crate::Size::default(),
            actual: crate::Size::new(300, 200),
        });
    }
    floating.handle_action(Action::ToggleFloatingFocus);
    measure("floating", floating);
    // Floating dialogs of a floating window (a floated app with two dialogs
    // open), the parent focused and so raised above them: the arrangement
    // then draws the dialogs above it (`floating_order.rs`), the path that
    // builds the drawing order.
    let mut dialogs = scene();
    add_floating_chain(&mut dialogs, &[None, Some(0), Some(0)]);
    dialogs.handle_action(Action::FocusWindowId(WindowId(WINDOWS + 1)));
    dialogs.handle_action(Action::ToggleFloatingFocus);
    assert_ne!(
        floating_ids(&dialogs, false),
        floating_ids(&dialogs, true),
        "the dialogs scene must draw in another order than it stacks"
    );
    measure("floating dialogs", dialogs);
    // The worst case a client can build for that path: 64 floating windows,
    // each the dialog of the one before, the bottom one focused (raised
    // over all of them).
    let mut chain = scene();
    let parents: Vec<Option<usize>> = (0..64).map(|i: usize| i.checked_sub(1)).collect();
    add_floating_chain(&mut chain, &parents);
    chain.handle_action(Action::FocusWindowId(WindowId(WINDOWS + 1)));
    chain.handle_action(Action::ToggleFloatingFocus);
    measure("floating chain of 64", chain);
    let mut long = scene();
    let parents: Vec<Option<usize>> = (0..1000).map(|i: usize| i.checked_sub(1)).collect();
    add_floating_chain(&mut long, &parents);
    long.handle_action(Action::FocusWindowId(WindowId(WINDOWS + 1)));
    long.handle_action(Action::ToggleFloatingFocus);
    measure("floating chain of 1000", long);
    // PR #243's re-review scene: a tiled chain (each window the transient
    // of the previous, set after it mapped so it stays tiled), as many
    // floating dialogs on the chain's tip, and an unrelated window
    // fullscreen over them -- the covering rule asks each dialog whether it
    // descends from the fullscreen window.
    for n in [250, 1000] {
        measure(
            &format!("covered tiled chain + {n} dialogs"),
            covered_chain(n),
        );
    }
}

/// The re-review scene above, with `n` tiled windows and `n` dialogs.
fn covered_chain(n: u64) -> World {
    let mut world = World::new(config());
    world.handle_event(Event::OutputAdded {
        id: OutputId(1),
        area: SCREEN,
    });
    for id in 1..=n {
        open(&mut world, id);
    }
    for id in 2..=n {
        world.handle_event(Event::WindowChanged {
            id: WindowId(id),
            info: WindowInfo {
                parent: Some(WindowId(id - 1)),
                ..WindowInfo::default()
            },
        });
    }
    let fullscreen = 100_000;
    open(&mut world, fullscreen);
    for k in 0..n {
        let id = 200_000 + k;
        open_with(
            &mut world,
            id,
            WindowInfo {
                parent: Some(WindowId(n)),
                ..WindowInfo::default()
            },
        );
        world.handle_event(Event::FloatingRequested {
            id: WindowId(id),
            floating: true,
            size: None,
        });
        world.handle_event(Event::FrameObserved {
            id: WindowId(id),
            requested: crate::Size::default(),
            actual: crate::Size::new(300, 200),
        });
    }
    world.handle_action(Action::FocusWindowId(WindowId(fullscreen)));
    world.handle_event(Event::FullscreenRequested {
        id: WindowId(fullscreen),
        fullscreen: true,
    });
    assert_eq!(world.fullscreen_on(OutputId(1)), Some(WindowId(fullscreen)));
    world
}

/// The focused workspace's floating windows: in stacking order, or in the
/// order the arrangement draws them.
fn floating_ids(world: &World, drawn: bool) -> Vec<WindowId> {
    if drawn {
        world
            .arrange()
            .placements
            .iter()
            .filter(|p| {
                p.floating
                    && world
                        .locate(p.id)
                        .is_some_and(|l| l.output == world.focused_output)
            })
            .map(|p| p.id)
            .collect()
    } else {
        world.outputs[world.focused_output]
            .active_workspace()
            .floating
            .clone()
    }
}

/// Opens one floating window per entry after the scene's windows, drawn
/// 300x200, each transient for the entry's earlier window (by position in
/// the list) when it names one.
fn add_floating_chain(world: &mut World, parents: &[Option<usize>]) {
    for (index, parent) in parents.iter().enumerate() {
        let id = WINDOWS + 1 + index as u64;
        let info = WindowInfo {
            parent: parent.map(|p| WindowId(WINDOWS + 1 + p as u64)),
            ..WindowInfo::default()
        };
        open_with(world, id, info);
        world.handle_event(Event::FloatingRequested {
            id: WindowId(id),
            floating: true,
            size: None,
        });
        world.handle_event(Event::FrameObserved {
            id: WindowId(id),
            requested: crate::Size::default(),
            actual: crate::Size::new(300, 200),
        });
    }
}
