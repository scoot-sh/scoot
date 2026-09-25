//! Outputs taken away and added while clients are connected (milestone 19,
//! phase E2): what `--tty` does when a monitor is unplugged or plugged in.
//!
//! `State::remove_output` and a runtime `headless::add_output` are
//! backend-independent, so the whole lifecycle is pinned here on headless
//! outputs with a real client holding objects on them -- far more of the
//! protocol surface than one physical replug on the Asahi machine can
//! check (that run is recorded in `Asahi.md` Test 3). Each test asserts what
//! the *client* was told, since a protocol error or a missing event is where
//! an output going away hurts.

use scoot_core::OutputId;

use super::per_output::{Ack, BAR_BGRA, CANVAS, Removals, Seen, Step, bar_on, draw, session};
use crate::compositor::headless;
use crate::compositor::test_support::contains;

fn removals(harness: &mut crate::compositor::test_support::Harness<Step, Ack>) -> Removals {
    match harness.run(Step::Removals) {
        Ack::Removals(removals) => removals,
        _ => panic!("expected a removals report"),
    }
}

/// Everything a client holds on the removed output is ended the protocol's
/// way -- the layer surface `closed`, the capture session `stopped`, the
/// gamma control `failed`, the `wl_output` global withdrawn -- and nothing on
/// the output that stays is touched.
#[test]
fn removing_an_output_ends_what_clients_hold_on_it_and_nothing_else() {
    let mut harness = session(2);
    let kept = bar_on(&mut harness, 0);
    let gone = bar_on(&mut harness, 1);
    let Ack::Held(kept_gamma) = harness.run(Step::GammaHold { output: 0 }) else {
        panic!("expected a held gamma control");
    };
    let Ack::Held(gone_gamma) = harness.run(Step::GammaHold { output: 1 }) else {
        panic!("expected a held gamma control");
    };
    harness.run(Step::CaptureNoWait { output: 1 });

    assert!(harness.state.remove_output(OutputId(2)));
    harness.settle();

    let report = removals(&mut harness);
    assert!(
        report.bars_closed[gone],
        "the removed output's bar is closed"
    );
    assert!(!report.bars_closed[kept], "the other output's bar is not");
    assert!(
        report.capture_stopped,
        "a capture of the removed output is stopped"
    );
    assert_eq!(
        report.outputs_removed,
        vec![1],
        "only the removed output's wl_output global is withdrawn"
    );
    let Ack::GammaState { failed, .. } = harness.run(Step::GammaState { held: gone_gamma }) else {
        panic!("expected a gamma state");
    };
    assert!(failed, "the removed output's gamma control failed");
    let Ack::GammaState { failed, .. } = harness.run(Step::GammaState { held: kept_gamma }) else {
        panic!("expected a gamma state");
    };
    assert!(!failed, "the other output's gamma control is untouched");

    // And the session carries on: one output, drawing its own bar.
    assert_eq!(harness.state.outputs.len(), 1);
    let pixels = draw(&mut harness, OutputId(1));
    assert!(
        contains(&pixels, BAR_BGRA),
        "the remaining output still draws"
    );
}

/// A client whose bind of the output's global was already in flight when it
/// went away must not die for it: the global is withdrawn, not destroyed,
/// until the grace period passes.
#[test]
fn a_bind_racing_the_removal_is_not_a_protocol_error() {
    let mut harness = session(2);
    // The client has seen both globals before anything goes away.
    assert_eq!(removals(&mut harness).outputs_announced, 2);
    assert!(harness.state.remove_output(OutputId(2)));
    harness.settle();
    match harness.run_or_disconnect(Step::Rebind { output: 1 }) {
        Ok(Ack::Done) => {}
        Ok(_) => panic!("unexpected answer"),
        Err(error) => panic!("binding a just-removed output killed the client: {error}"),
    }
}

/// The last output is never taken away: with none the core would park every
/// window as unplaced and nothing could draw.
#[test]
fn the_last_output_is_never_removed() {
    let mut harness = session(1);
    assert!(!harness.state.remove_output(OutputId(1)));
    assert_eq!(harness.state.outputs.len(), 1);
    assert_eq!(removals(&mut harness).outputs_removed, Vec::<usize>::new());
}

/// An unknown id is refused, not a panic, and changes nothing.
#[test]
fn removing_an_unknown_output_changes_nothing() {
    let mut harness = session(2);
    assert!(!harness.state.remove_output(OutputId(9)));
    assert_eq!(harness.state.outputs.len(), 2);
}

/// The outputs to the right of a removed one close the gap: removing the
/// middle of three moves the third to where the second was, in the `Space`,
/// in the core and on the wire (`wl_output` geometry is what the client
/// reads, and `output_geometry` is what input and rendering read).
#[test]
fn outputs_right_of_a_removed_one_are_repacked() {
    let mut harness = session(3);
    assert!(harness.state.remove_output(OutputId(2)));
    harness.settle();
    let third = harness
        .state
        .outputs
        .get(OutputId(3))
        .cloned()
        .expect("the third output remains");
    let geometry = harness.state.space.output_geometry(&third).expect("mapped");
    assert_eq!(
        (geometry.loc.x, geometry.loc.y),
        (CANVAS, 0),
        "the third output moved left into the gap"
    );
    assert_eq!(
        third.current_location(),
        (CANVAS, 0).into(),
        "and the wire says so"
    );
    let area = harness
        .state
        .world
        .outputs()
        .into_iter()
        .find(|(id, _)| *id == OutputId(3))
        .map(|(_, area)| area)
        .expect("the core knows the third output");
    assert_eq!((area.x, area.y), (CANVAS, 0), "and so does the core");
}

/// A pointer resting on an output that goes away is brought back inside the
/// desktop rather than left over nothing.
#[test]
fn a_pointer_on_a_removed_output_comes_back_inside() {
    let mut harness = session(2);
    harness.state.pointer_move(f64::from(CANVAS) * 1.5, 50.0);
    assert!(harness.state.remove_output(OutputId(2)));
    let pointer = harness.state.seat.get_pointer().expect("a pointer");
    let location = pointer.current_location();
    assert!(
        location.x < f64::from(CANVAS) && location.y < f64::from(CANVAS),
        "the pointer is back on the remaining output, at {location:?}"
    );
}

/// An output added while a client is connected is announced to it, gets a
/// render target and a strip of its own, and draws what is mapped on it --
/// the plug-in half of the lifecycle.
#[test]
fn an_output_added_at_runtime_is_announced_and_drawn() {
    let mut harness = session(1);
    let id = headless::add_output(&mut harness.state, "headless-2", CANVAS, CANVAS)
        .expect("a runtime output");
    harness.settle();
    assert_eq!(removals(&mut harness).outputs_announced, 2);
    bar_on(&mut harness, 1);
    let pixels = draw(&mut harness, id);
    assert!(
        contains(&pixels, BAR_BGRA),
        "the new output draws its own bar"
    );
    assert!(
        !contains(&draw(&mut harness, OutputId(1)), BAR_BGRA),
        "and only there"
    );
}

/// Unplug and plug back in: the output comes back under a fresh id (ids are
/// never reused), and the client sees a second global rather than the old
/// one resurrected.
#[test]
fn an_output_removed_then_added_again_is_a_new_output() {
    let mut harness = session(2);
    assert_eq!(removals(&mut harness).outputs_announced, 2);
    assert!(harness.state.remove_output(OutputId(2)));
    let id = headless::add_output(&mut harness.state, "headless-2", CANVAS, CANVAS)
        .expect("the output plugged back in");
    harness.settle();
    assert_eq!(id, OutputId(3), "a fresh id, never the removed one");
    let report = removals(&mut harness);
    assert_eq!(report.outputs_removed, vec![1]);
    assert_eq!(report.outputs_announced, 3, "a new global was announced");
}

/// A window on a removed output is told `wl_surface.leave` for it *before*
/// the output's `wl_output` global is withdrawn: after the `global_remove` a
/// client can no longer resolve the output a leave names (foot logged
/// "unmapped from unknown output" on the Asahi replug before this).
/// Fail-first: without the immediate `Space` refresh the leave arrives at the
/// next frame, after the `global_remove`.
#[test]
fn a_windows_leave_precedes_the_outputs_global_remove() {
    let mut harness = session(2);
    harness.run(Step::Window);
    harness
        .state
        .act(scoot_core::Action::MoveFocusedWindowToOutput(OutputId(2)));
    harness.state.render();
    harness.settle();
    let Ack::Order(before) = harness.run(Step::Order) else {
        panic!("expected the event log");
    };
    assert!(
        before.contains(&Seen::Enter(1)),
        "the setup put the window on the second output: {before:?}"
    );

    assert!(harness.state.remove_output(OutputId(2)));
    harness.settle();
    let Ack::Order(after) = harness.run(Step::Order) else {
        panic!("expected the event log");
    };
    let leave = after
        .iter()
        .rposition(|seen| *seen == Seen::Leave(1))
        .expect("the window is told it left the removed output");
    let removed = after
        .iter()
        .position(|seen| *seen == Seen::GlobalRemove(1))
        .expect("the removed output's global is withdrawn");
    assert!(
        leave < removed,
        "leave must precede global_remove: {after:?}"
    );
}
