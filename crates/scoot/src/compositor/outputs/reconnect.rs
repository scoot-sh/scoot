//! A monitor going away and coming back (milestone 19, phase E2, on real
//! hardware): `State::remove_output` adopts the removed output's workspaces
//! onto the remaining output, and an add under the same connector identity
//! moves the still-open ones back.
//!
//! The ticket's three harness pins, on headless outputs with a real client
//! holding windows on them (identity is name-only here -- the EDID half is
//! pinned by `output_identity.rs`'s own tests):
//!
//! 1. remove output 2 holding two workspaces, add an output with the same
//!    identity, and the windows and workspaces are back in order with the
//!    active index restored -- and the default positional bind reaches the
//!    returned monitor;
//! 2. add an output with a *different* identity, and nothing moves (the
//!    record survives for the matching add that comes later);
//! 3. a window moved by hand in between stays where it was put.

use scoot_core::{Action, OutputId, Vertical, WindowId};

use super::per_output::{Ack, CANVAS, Step, session};
use crate::compositor::headless;
use crate::compositor::test_support::Harness;

/// Move the pointer onto the second output: windows mapped after this open
/// there (new-window placement follows the pointer).
fn point_at_second(harness: &mut Harness<Step, Ack>) {
    harness.state.pointer_move(f64::from(CANVAS) * 1.5, 50.0);
}

/// Map one xdg toplevel and answer the window the compositor focused for it.
fn map_window(harness: &mut Harness<Step, Ack>) -> WindowId {
    harness.run(Step::Window);
    harness.settle();
    harness.state.focus.expect("a mapped window takes focus")
}

/// The output the core placed `id` on.
fn output_of(harness: &Harness<Step, Ack>, id: WindowId) -> OutputId {
    harness
        .state
        .world
        .arrange()
        .get(id)
        .expect("the window is placed")
        .output
}

/// Output 2 holding two workspaces -- ws0 `[a][b]`, ws1 `[c]`, ws1 active --
/// with output 1 empty. Returns the three window ids in mapping order.
fn two_workspaces_on_second(harness: &mut Harness<Step, Ack>) -> (WindowId, WindowId, WindowId) {
    point_at_second(harness);
    let a = map_window(harness);
    let b = map_window(harness);
    let c = map_window(harness);
    assert!(
        a != b && b != c && a != c,
        "each mapping focused a new window"
    );
    // Carry the focused window (c) down into the trailing empty: ws1 `[c]`.
    harness
        .state
        .act(Action::MoveWindowToWorkspace(Vertical::Down));
    assert_eq!(output_of(harness, a), OutputId(2));
    assert_eq!(output_of(harness, b), OutputId(2));
    assert_eq!(output_of(harness, c), OutputId(2));
    (a, b, c)
}

/// Pin 1: remove output 2 holding two workspaces, add an output with the
/// same identity, and the windows and workspaces are back in order with the
/// active index restored -- and the default positional bind reaches it.
#[test]
fn removing_then_re_adding_an_output_restores_its_workspaces() {
    let mut harness = session(2);
    let (a, b, c) = two_workspaces_on_second(&mut harness);

    assert!(harness.state.remove_output(OutputId(2)));
    harness.settle();
    for id in [a, b, c] {
        assert_eq!(output_of(&harness, id), OutputId(1));
    }

    let id = headless::add_output(&mut harness.state, "headless-2", CANVAS, CANVAS)
        .expect("the output plugged back in");
    harness.settle();
    assert_eq!(id, OutputId(3), "a fresh id, never the removed one");

    // The same workspaces, active index and column order.
    let arrangement = harness.state.world.arrange();
    for window in [a, b, c] {
        assert_eq!(
            arrangement.get(window).expect("placed").output,
            OutputId(3),
            "window {window:?} is back on the returned output"
        );
    }
    let workspaces = harness
        .state
        .world
        .workspaces(OutputId(3))
        .expect("the returned output has workspaces");
    assert_eq!(workspaces.count, 3, "ws0, ws1 and the trailing empty");
    assert_eq!(workspaces.active, 1, "the active index is restored");
    let at = |window: WindowId| arrangement.get(window).expect("placed").rect;
    // ws1 (c) is active: it shows, ws0 (a, b) does not.
    assert!(arrangement.get(c).expect("placed").visible);
    assert!(!arrangement.get(a).expect("placed").visible);
    assert!(!arrangement.get(b).expect("placed").visible);
    // Column order within ws0: a left of b.
    assert!(
        at(a).x < at(b).x,
        "ws0 keeps its column order: {at_a:?} vs {at_b:?}",
        at_a = at(a),
        at_b = at(b)
    );

    // The default output-2 bind positionally reaches the returned monitor:
    // position 1 is output 3 now, whatever id it carries.
    harness.state.act(Action::FocusOutputIndex(1));
    assert_eq!(
        harness.state.world.focused_output(),
        Some(OutputId(3)),
        "Super+period reaches the returned monitor"
    );
    // And carrying a window there lands on it too.
    harness.state.act(Action::FocusWindowId(a));
    harness.state.act(Action::MoveFocusedWindowToOutputIndex(1));
    assert_eq!(output_of(&harness, a), OutputId(3));
}

/// Pin 2: an output added under a *different* identity moves nothing -- and
/// the filed record survives for the matching add that comes later.
#[test]
fn adding_an_output_with_a_different_identity_moves_nothing() {
    let mut harness = session(2);
    let (a, b, c) = two_workspaces_on_second(&mut harness);
    assert!(harness.state.remove_output(OutputId(2)));
    harness.settle();

    // A different monitor on the same cable (or a new virtual output): its
    // windows stay adopted.
    let other = headless::add_output(&mut harness.state, "headless-9", CANVAS, CANVAS)
        .expect("an output with another identity");
    harness.settle();
    for id in [a, b, c] {
        assert_eq!(
            output_of(&harness, id),
            OutputId(1),
            "window {id:?} stays adopted"
        );
    }

    // The record survived the miss: the matching add still restores.
    let id = headless::add_output(&mut harness.state, "headless-2", CANVAS, CANVAS)
        .expect("the output plugged back in");
    harness.settle();
    assert_eq!(id, OutputId(4));
    for window in [a, b, c] {
        assert_eq!(
            output_of(&harness, window),
            OutputId(4),
            "window {window:?} is back once its own monitor returns"
        );
    }
    assert_ne!(other, id);
}

/// Pin 3: a window moved by hand in between stays where it was put, while
/// its old neighbours still go back.
#[test]
fn a_window_moved_by_hand_between_remove_and_add_stays() {
    let mut harness = session(2);
    // One window on the panel first, so the hand move has somewhere to go.
    let kept = map_window(&mut harness);
    assert_eq!(output_of(&harness, kept), OutputId(1));
    let (a, b, c) = two_workspaces_on_second(&mut harness);

    assert!(harness.state.remove_output(OutputId(2)));
    harness.settle();
    // Carry c by hand onto the panel's own first workspace, beside `kept`.
    harness.state.act(Action::FocusWindowId(c));
    harness.state.act(Action::MoveWindowToWorkspaceIndex(0));
    assert_eq!(output_of(&harness, c), OutputId(1));

    headless::add_output(&mut harness.state, "headless-2", CANVAS, CANVAS)
        .expect("the output plugged back in");
    harness.settle();

    assert_eq!(
        output_of(&harness, c),
        OutputId(1),
        "the hand-moved window stays where it was put"
    );
    for window in [a, b] {
        assert_eq!(
            output_of(&harness, window),
            OutputId(3),
            "window {window:?} is back on the returned output"
        );
    }
    assert_eq!(output_of(&harness, kept), OutputId(1));
}
