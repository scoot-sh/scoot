//! Random sequences of events and actions, checking the tree's invariants after
//! every step.

use super::*;
use crate::{Action, Horizontal, Size, SizeHints, Vertical};

/// xorshift64*: deterministic and dependency-free.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn size(&mut self, max: usize) -> i32 {
        self.below(max) as i32
    }
}

fn random_info(rng: &mut Rng) -> WindowInfo {
    let min = if rng.chance(30) {
        Size::new(rng.size(1200), rng.size(900))
    } else {
        Size::default()
    };
    WindowInfo {
        hints: SizeHints { min },
        ..WindowInfo::default()
    }
}

fn random_area(rng: &mut Rng) -> Rect {
    Rect::new(
        rng.size(4000),
        0,
        300 + rng.size(1700),
        200 + rng.size(1000),
    )
}

fn random_action(rng: &mut Rng, windows: &[WindowId]) -> Action {
    let horizontal = if rng.chance(50) {
        Horizontal::Left
    } else {
        Horizontal::Right
    };
    let vertical = if rng.chance(50) {
        Vertical::Up
    } else {
        Vertical::Down
    };
    match rng.below(10) {
        0 => Action::FocusColumn(horizontal),
        1 => Action::FocusWindow(vertical),
        2 => Action::MoveColumn(horizontal),
        3 => Action::MoveWindow(vertical),
        4 => Action::ConsumeOrExpel(horizontal),
        5 => Action::CycleColumnWidth,
        6 => Action::FocusWorkspace(vertical),
        7 => Action::MoveWindowToWorkspace(vertical),
        8 if !windows.is_empty() => Action::FocusWindowId(windows[rng.below(windows.len())]),
        _ => Action::CloseFocused,
    }
}

fn random_step(world: &mut World, rng: &mut Rng, next_id: &mut u64) {
    let windows: Vec<WindowId> = world.windows().into_iter().map(|(id, _)| id).collect();
    let outputs: Vec<OutputId> = world.outputs().into_iter().map(|(id, _)| id).collect();
    *next_id += 1;
    let event = match rng.below(12) {
        0 | 1 => Event::WindowOpened {
            id: WindowId(*next_id),
            info: random_info(rng),
            output: None,
            focus: rng.chance(80),
        },
        2 if !windows.is_empty() => Event::WindowClosed {
            id: windows[rng.below(windows.len())],
        },
        3 if rng.chance(20) => Event::OutputAdded {
            id: OutputId(*next_id),
            area: random_area(rng),
        },
        4 if !outputs.is_empty() && rng.chance(15) => Event::OutputRemoved {
            id: outputs[rng.below(outputs.len())],
        },
        5 if !outputs.is_empty() => Event::OutputChanged {
            id: outputs[rng.below(outputs.len())],
            area: random_area(rng),
        },
        6 if !windows.is_empty() => Event::FrameObserved {
            id: windows[rng.below(windows.len())],
            requested: Size::new(rng.size(2000), rng.size(1200)),
            actual: Size::new(rng.size(6000), rng.size(4000)),
        },
        7 if !windows.is_empty() => Event::FocusObserved {
            id: windows[rng.below(windows.len())],
        },
        8 if !windows.is_empty() => Event::WindowChanged {
            id: windows[rng.below(windows.len())],
            info: random_info(rng),
        },
        _ => {
            world.handle_action(random_action(rng, &windows));
            return;
        }
    };
    world.handle_event(event);
}

fn assert_invariants(world: &World) {
    let mut placed = world.unplaced.clone();
    for output in &world.outputs {
        let count = output.workspaces.len();
        assert!(output.active < count, "active workspace out of range");
        assert!(
            output.workspaces[count - 1].is_empty(),
            "last workspace must be empty"
        );
        for (index, ws) in output.workspaces.iter().enumerate() {
            assert!(
                !ws.is_empty() || index == output.active || index + 1 == count,
                "stray empty workspace at {index}"
            );
            assert!(
                ws.is_empty() || ws.focused < ws.columns.len(),
                "focused column out of range"
            );
            for column in &ws.columns {
                assert!(!column.windows.is_empty(), "empty column");
                assert!(
                    column.focused < column.windows.len(),
                    "focused window out of range"
                );
                assert!(
                    column.preset < world.config.column_widths.len(),
                    "preset out of range"
                );
                placed.extend(column.windows.iter().copied());
            }
        }
    }
    placed.sort();
    let mut known: Vec<WindowId> = world.windows.keys().copied().collect();
    known.sort();
    assert_eq!(
        placed, known,
        "every window must be in the tree exactly once"
    );
    assert!(world.outputs.is_empty() || world.focused_output < world.outputs.len());
    for placement in world.arrange().placements {
        assert!(
            placement.rect.w >= 1 && placement.rect.h >= 1,
            "{placement:?}"
        );
    }
}

#[test]
fn random_sequences_keep_the_tree_consistent() {
    for seed in 1..=24 {
        let mut rng = Rng(seed);
        let mut world = World::new(config());
        let mut next_id = 0;
        for step in 0..1500 {
            random_step(&mut world, &mut rng, &mut next_id);
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| assert_invariants(&world)))
                .is_err()
            {
                panic!("invariant broken with seed {seed} at step {step}");
            }
        }
    }
}
