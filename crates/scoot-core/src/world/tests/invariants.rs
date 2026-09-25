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

fn random_info(rng: &mut Rng, windows: &[WindowId]) -> WindowInfo {
    let min = if rng.chance(30) {
        Size::new(rng.size(1200), rng.size(900))
    } else {
        Size::default()
    };
    // Sometimes transient for another window -- usually a live one,
    // sometimes a stale or wild id, and now and then itself.
    let parent = rng.chance(30).then(|| random_window(rng, windows));
    WindowInfo {
        hints: SizeHints { min },
        parent,
        ..WindowInfo::default()
    }
}

/// A size a platform might ask a floating window for: usually plausible,
/// sometimes zero, negative or i32-extreme (a window rule's numbers are the
/// user's, and a future platform's may be anyone's).
fn random_float_size(rng: &mut Rng) -> Option<Size> {
    match rng.below(6) {
        0 => None,
        1 => Some(Size::new(i32::MAX, i32::MAX)),
        2 => Some(Size::new(0, -5)),
        _ => Some(Size::new(rng.size(3000), rng.size(2000))),
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
    match rng.below(20) {
        0 => Action::FocusColumn(horizontal),
        1 => Action::FocusWindow(vertical),
        2 => Action::MoveColumn(horizontal),
        3 => Action::MoveWindow(vertical),
        4 => Action::ConsumeOrExpel(horizontal),
        5 => Action::CycleColumnWidth,
        6 => Action::FocusWorkspace(vertical),
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
        14 => Action::ToggleFullscreen,
        // By id, off the same wire as the others: usually a live window,
        // sometimes a stale or wild one.
        15 => Action::SetFullscreen {
            id: random_window(rng, windows),
            fullscreen: rng.chance(60),
        },
        16 => Action::ToggleFloating,
        17 => Action::SetFloating {
            id: random_window(rng, windows),
            floating: rng.chance(60),
        },
        18 => Action::ToggleFloatingFocus,
        _ => Action::CloseFocused,
    }
}

/// Usually one of the live windows, sometimes a stale or wild id -- a window
/// id comes off the IPC and Wayland wires as an unbounded number.
fn random_window(rng: &mut Rng, windows: &[WindowId]) -> WindowId {
    match rng.below(8) {
        0 => WindowId(u64::MAX),
        1 => WindowId(u64::MAX / 2),
        _ if !windows.is_empty() => windows[rng.below(windows.len())],
        _ => WindowId(1),
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

/// One step of the randomized sequence: an event a platform might report or
/// an action a user or agent might ask for.
#[derive(Debug)]
enum Step {
    Event(Event),
    Action(Action),
}

impl Step {
    /// Whether this step, applied to `world`, can only touch floating
    /// windows -- so every tiled window's rect must come out of it exactly
    /// as it went in ("floating windows never change the strip").
    fn floating_only(&self, world: &World) -> bool {
        let floating = |id: &WindowId| world.is_floating(*id) && !world.is_fullscreen(*id);
        // Focusing a floating window elsewhere switches workspace or output,
        // which re-scrolls that strip: not floating-only.
        let here = |id: &WindowId| {
            floating(id)
                && world.locate(*id).is_some_and(|loc| {
                    loc.output == world.focused_output
                        && loc.workspace == world.outputs[loc.output].active
                })
        };
        match self {
            Step::Event(Event::FrameObserved { id, .. }) => floating(id),
            Step::Event(Event::FocusObserved { id }) => here(id),
            Step::Action(Action::FocusWindowId(id)) => here(id),
            Step::Action(Action::ToggleFloatingFocus) => true,
            Step::Action(Action::FocusWindow(_)) => world.floating_has_focus(),
            _ => false,
        }
    }

    /// Whether this step, applied to `world`, must leave
    /// `World::focused_window` alone: `set-floating` on a window that is not
    /// the focused one (it never moves focus), and a window opening without
    /// focus -- while some window has focus to keep (a window opening into
    /// a workspace with nothing on it is that workspace's focus, there being
    /// nothing else).
    fn keeps_focus(&self, world: &World) -> bool {
        if world.focused_window().is_none() {
            return false;
        }
        match self {
            Step::Action(Action::SetFloating { id, .. }) => world.focused_window() != Some(*id),
            Step::Event(Event::WindowOpened { focus, .. }) => !focus,
            _ => false,
        }
    }
}

fn random_step(world: &mut World, rng: &mut Rng, next_id: &mut u64) -> Step {
    let windows: Vec<WindowId> = world.windows().into_iter().map(|(id, _)| id).collect();
    let outputs: Vec<OutputId> = world.outputs().into_iter().map(|(id, _)| id).collect();
    *next_id += 1;
    let event = match rng.below(16) {
        0 | 1 => Event::WindowOpened {
            id: WindowId(*next_id),
            info: random_info(rng, &windows),
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
            info: random_info(rng, &windows),
        },
        9 if !outputs.is_empty() => Event::OutputUsableAreaChanged {
            id: outputs[rng.below(outputs.len())],
            area: random_usable_area(rng),
        },
        10 => Event::FullscreenRequested {
            id: random_window(rng, &windows),
            fullscreen: rng.chance(60),
        },
        // Floating as a window maps: most often the one that just opened,
        // which is the shape a platform produces.
        11 | 12 => Event::FloatingRequested {
            id: if rng.chance(50) {
                WindowId(*next_id - 1)
            } else {
                random_window(rng, &windows)
            },
            floating: rng.chance(80),
            size: random_float_size(rng),
        },
        _ => return Step::Action(random_action(rng, &windows, &outputs)),
    };
    Step::Event(event)
}

fn apply_step(world: &mut World, step: Step) {
    match step {
        Step::Event(event) => world.handle_event(event),
        Step::Action(action) => {
            world.handle_action(action);
        }
    }
}

/// Every tiled window's rect, in arrangement order.
fn tiled_rects(world: &World) -> Vec<(WindowId, Rect)> {
    world
        .arrange()
        .placements
        .iter()
        .filter(|p| !p.floating)
        .map(|p| (p.id, p.rect))
        .collect()
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
                ws.columns.is_empty() || ws.focused < ws.columns.len(),
                "focused column out of range"
            );
            // The floating layer's focus flag never outlives the layer.
            assert!(
                !ws.floating_focused || !ws.floating.is_empty(),
                "floating focus on an empty floating layer"
            );
            for id in &ws.floating {
                assert!(
                    world.is_floating(*id),
                    "{id:?} in a floating layer, not floating"
                );
                placed.push(*id);
            }
            for column in &ws.columns {
                assert!(!column.windows.is_empty(), "empty column");
                // A fullscreen window is always its column's focused window
                // -- and so there is at most one per column.
                for (index, id) in column.windows.iter().enumerate() {
                    assert!(
                        index == column.focused || !world.is_fullscreen(*id),
                        "fullscreen window {id:?} is not its column's focused one"
                    );
                }
                assert!(
                    column.focused < column.windows.len(),
                    "focused window out of range"
                );
                assert!(
                    column.preset < world.config.column_widths.len(),
                    "preset out of range"
                );
                for id in &column.windows {
                    assert!(
                        !world.is_floating(*id),
                        "floating window {id:?} in a column"
                    );
                }
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
    let arrangement = world.arrange();
    for placement in &arrangement.placements {
        assert!(
            placement.rect.w >= 1 && placement.rect.h >= 1,
            "{placement:?}"
        );
        assert_eq!(
            placement.fullscreen,
            world.is_fullscreen(placement.id),
            "{placement:?}"
        );
        assert_eq!(
            placement.floating,
            world.is_floating(placement.id),
            "{placement:?}"
        );
        // What the core asks a window for is its rect for everything the
        // layout sizes, and never bigger than the output for a floating one.
        if !placement.floating || placement.fullscreen {
            assert_eq!(
                placement.requested,
                Some(placement.rect.size()),
                "{placement:?}"
            );
        }
        if let Some(requested) = placement.requested {
            assert!(requested.w >= 1 && requested.h >= 1, "{placement:?}");
        }
        // A visible floating window lies inside its output's usable area.
        if placement.floating && placement.visible && !placement.fullscreen {
            let usable = world
                .outputs
                .iter()
                .find(|o| o.id == placement.output)
                .map(|o| o.usable)
                .expect("placed on a known output");
            assert_eq!(
                placement.rect.intersection(usable),
                placement.rect,
                "{placement:?} escapes {usable:?}"
            );
        }
    }
    // A covering window covers its output exactly, and nothing else on that
    // output shows. (`Rect::new` sizes are floored at 1 in the placement, so
    // compare against the non-degenerate outputs `random_area` produces.)
    for output in &world.outputs {
        let Some(covering) = world.fullscreen_on(output.id) else {
            continue;
        };
        for placement in arrangement
            .placements
            .iter()
            .filter(|p| p.output == output.id)
        {
            if placement.id == covering {
                assert!(placement.visible, "covering {placement:?} is not visible");
                assert_eq!(placement.rect, output.area, "covering {placement:?}");
            } else if placement.floating
                && !placement.fullscreen
                && world.descends_from(placement.id, covering)
            {
                // Its own dialogs stay up, above it.
                if placement.visible {
                    let at = |id| arrangement.placements.iter().position(|p| p.id == id);
                    assert!(
                        at(placement.id) > at(covering),
                        "{placement:?} under its parent"
                    );
                }
            } else {
                assert!(
                    !placement.visible,
                    "{placement:?} shows beside {covering:?}"
                );
            }
        }
    }
    // Nothing visible in the strip overlaps anything else visible in it on
    // the same output -- in particular a fullscreen window scrolled beside
    // the focused column must keep to its own slot. (Floating windows are
    // above the strip by design.)
    let visible: Vec<_> = arrangement
        .placements
        .iter()
        .filter(|p| p.visible && !p.floating)
        .collect();
    for (i, a) in visible.iter().enumerate() {
        for b in &visible[i + 1..] {
            if a.output == b.output {
                let overlap = a.rect.intersection(b.rect);
                assert!(overlap.w == 0 || overlap.h == 0, "{a:?} overlaps {b:?}");
            }
        }
    }
    // And `fullscreen_on` is exactly "the active workspace's focused window
    // is fullscreen" -- nothing covers that should not.
    for output in &world.outputs {
        let focused = output.active_workspace().focused_window();
        let expected = focused.filter(|id| world.is_fullscreen(*id));
        assert_eq!(world.fullscreen_on(output.id), expected);
    }
}

#[test]
fn random_sequences_keep_the_tree_consistent() {
    // How many steps ended with some output covered by a fullscreen window,
    // so the fullscreen invariants above are known to have been exercised
    // rather than passing vacuously.
    let mut covered_steps = 0;
    // Likewise for floating: steps ending with a floating window on screen,
    // steps that ended with the floating layer focused, and floating-only
    // steps whose strip was compared before and after.
    let (mut floating_steps, mut floating_focus_steps, mut strip_checks) = (0, 0, 0);
    let mut focus_checks = 0;
    for seed in 1..=24 {
        let mut rng = Rng(seed);
        let mut world = World::new(config());
        let mut next_id = 0;
        for step in 0..1500 {
            let random = random_step(&mut world, &mut rng, &mut next_id);
            let strip_before = random
                .floating_only(&world)
                .then(|| (tiled_rects(&world), format!("{random:?}")));
            let focus_before = random
                .keeps_focus(&world)
                .then(|| (world.focused_window(), format!("{random:?}")));
            apply_step(&mut world, random);
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| assert_invariants(&world)))
                .is_err()
            {
                panic!("invariant broken with seed {seed} at step {step}");
            }
            if let Some((before, what)) = focus_before {
                assert_eq!(
                    world.focused_window(),
                    before,
                    "{what} moved focus (seed {seed}, step {step})"
                );
                focus_checks += 1;
            }
            if let Some((before, what)) = strip_before {
                assert_eq!(
                    tiled_rects(&world),
                    before,
                    "{what} moved the strip (seed {seed}, step {step})"
                );
                strip_checks += 1;
            }
            if world
                .outputs
                .iter()
                .any(|output| world.fullscreen_on(output.id).is_some())
            {
                covered_steps += 1;
            }
            if world
                .arrange()
                .placements
                .iter()
                .any(|p| p.floating && p.visible)
            {
                floating_steps += 1;
            }
            if world.floating_has_focus() {
                floating_focus_steps += 1;
            }
        }
    }
    assert!(
        covered_steps > 1000,
        "only {covered_steps} steps had a covering fullscreen window"
    );
    assert!(
        floating_steps > 1000,
        "only {floating_steps} steps had a visible floating window"
    );
    assert!(
        floating_focus_steps > 1000,
        "only {floating_focus_steps} steps had the floating layer focused"
    );
    assert!(
        strip_checks > 500,
        "only {strip_checks} floating-only steps were checked"
    );
    assert!(
        focus_checks > 500,
        "only {focus_checks} focus-keeping steps were checked"
    );
}
