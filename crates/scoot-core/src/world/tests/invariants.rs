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

/// A usable area as a platform might report one: usually a plausible strip
/// taken off an edge, sometimes a degenerate or i32-extreme rectangle, since
/// the numbers behind it are a client's own (a layer surface's exclusive zone
/// and margins are raw `i32`s off the wire).
fn random_usable_area(rng: &mut Rng) -> Rect {
    match rng.below(10) {
        0 => Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX),
        1 => Rect::new(i32::MAX, i32::MAX, i32::MAX, i32::MAX),
        2 => Rect::new(0, 0, 0, 0),
        3 => Rect::new(rng.size(2000), rng.size(2000), -1, -1),
        _ => Rect::new(
            rng.size(4000),
            rng.size(1000),
            rng.size(4000),
            rng.size(1500),
        ),
    }
}

fn random_action(rng: &mut Rng, windows: &[WindowId], outputs: &[OutputId]) -> Action {
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
    match rng.below(15) {
        0 => Action::FocusColumn(horizontal),
        1 => Action::FocusWindow(vertical),
        2 => Action::MoveColumn(horizontal),
        3 => Action::MoveWindow(vertical),
        4 => Action::ConsumeOrExpel(horizontal),
        5 => Action::CycleColumnWidth,        6 => Action::FocusWorkspace(vertical),
        7 => Action::MoveWindowToWorkspace(vertical),
        8 if !windows.is_empty() => Action::FocusWindowId(windows[rng.below(windows.len())]),
        // Usually a plausible position, sometimes a wild one: this index
        // comes off a wire (`ext-workspace-v1`'s `activate`), so "a client
        // asked for workspace 2^63" has to be as ordinary as "workspace 1".
        9 => Action::FocusWorkspaceIndex(match rng.below(8) {
            0 => usize::MAX,
            1 => usize::MAX / 2,
            other => other,
        }),
        // The move half takes the same shape: an absolute index off the
        // same wire, wild values included, and the window must survive all
        // of them (the out-of-range case leaves it where it was).
        10 => Action::MoveWindowToWorkspaceIndex(match rng.below(8) {
            0 => usize::MAX,
            1 => usize::MAX / 2,
            other => other,
        }),
        // The cross-output halves: usually a live output, sometimes a stale
        // or wild id off the same wire -- and the window must survive all of
        // them (an unknown id leaves it where it was, and focus stays valid).
        11 => Action::MoveFocusedWindowToOutput(random_output(rng, outputs)),
        12 => Action::FocusOutput(random_output(rng, outputs)),
        // The absolute width half: usually a plausible position, sometimes a
        // wild one off the same wire -- and the column must survive all of
        // them (an out-of-range index is ignored, and the stored preset stays
        // a valid index into the width list).
        13 => Action::SetColumnWidth(match rng.below(8) {
            0 => usize::MAX,
            1 => usize::MAX / 2,
            other => other,
        }),
        _ => Action::CloseFocused,
    }
}

/// Usually one of the live outputs, sometimes a plausible-but-unknown id or
/// a wild one: an output id comes off the IPC wire as an unbounded number,
// like a workspace index.
fn random_output(rng: &mut Rng, outputs: &[OutputId]) -> OutputId {
    match rng.below(8) {
        0 => OutputId(u64::MAX),
        1 => OutputId(u64::MAX / 2),
        2 => OutputId(999),
        _ if !outputs.is_empty() => outputs[rng.below(outputs.len())],
        _ => OutputId(1),
    }
}

fn random_step(world: &mut World, rng: &mut Rng, next_id: &mut u64) {
    let windows: Vec<WindowId> = world.windows().into_iter().map(|(id, _)| id).collect();
    let outputs: Vec<OutputId> = world.outputs().into_iter().map(|(id, _)| id).collect();
    *next_id += 1;
    let event = match rng.below(13) {
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
        9 if !outputs.is_empty() => Event::OutputUsableAreaChanged {
            id: outputs[rng.below(outputs.len())],
            area: random_usable_area(rng),
        },
        _ => {
            world.handle_action(random_action(rng, &windows, &outputs));
            return;
        }
    };
    world.handle_event(event);
}

fn assert_invariants(world: &World) {
    let mut placed = world.unplaced.clone();
    for output in &world.outputs {
        // Whatever a platform reported, an output's usable area is always a
        // sub-rectangle of the output itself -- otherwise the layout would be
        // placing windows off the screen it thinks it is filling.
        assert_eq!(
            output.usable,
            output.usable.intersection(output.area),
            "usable area escaped its output"
        );
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
