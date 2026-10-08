//! A scale per output (`[[outputs]]`, see `output_config.rs`): each output
//! runs at its own scale, a surface is told the scale of the output it
//! belongs to -- re-told when it moves -- and a reload re-decides each
//! output's scale on its own.
//!
//! Two outputs, `CANVAS` physical pixels square each: `headless` at the
//! session default and `headless-2` wherever its entry puts it. Every
//! assertion about what a client heard reads the live client's own record of
//! the events (`Ack::Scales`), not the server's intent.

use scoot_core::{Action, OutputId};

use super::*;

/// `headless-2` at 2, the first output at the session default.
const SECOND_AT_2: &str = "[[outputs]]\nname = \"headless-2\"\nscale = 2.0\n";

/// Each output's name, scale and `wl_output.scale` integer, in creation
/// order: the one place an output's scale lives.
fn scales(fixture: &Fixture) -> Vec<(String, f64, i32)> {
    fixture
        .state
        .outputs
        .iter()
        .map(|output| {
            (
                output.name(),
                scale_of(output),
                output.current_scale().integer_scale(),
            )
        })
        .collect()
}

/// The two scale values the live client last heard for its surface.
fn heard(fixture: &mut Fixture) -> (Option<f64>, Option<i32>) {
    let Ack::Scales {
        preferred_scale,
        preferred_buffer_scale,
        ..
    } = fixture.run(Step::ReportScales)
    else {
        panic!("the report step must report what it saw");
    };
    (preferred_scale, preferred_buffer_scale)
}

/// Where output `id` sits in the layout, in logical pixels.
fn geometry(fixture: &Fixture, id: u64) -> (i32, i32, i32, i32) {
    let output = fixture.state.outputs.get(OutputId(id)).expect("the output");
    let geometry = fixture
        .state
        .space
        .output_geometry(output)
        .expect("a mapped output");
    (
        geometry.loc.x,
        geometry.loc.y,
        geometry.size.w,
        geometry.size.h,
    )
}

#[test]
fn each_output_runs_at_its_own_scale() {
    let fixture = Fixture::with_outputs(1.0, SECOND_AT_2, &["headless-2"]);
    assert_eq!(
        scales(&fixture),
        [
            ("headless".to_owned(), 1.0, 1),
            ("headless-2".to_owned(), 2.0, 2),
        ]
    );
    // The output without an entry is exactly what it always was: scale 1.0
    // spelled as the integer variant, not `Fractional(1.0)`.
    let first = fixture.state.outputs.primary().expect("the first output");
    assert!(matches!(first.current_scale(), Scale::Integer(1)));
    // Each laid out at its own logical size, side by side with no gap.
    assert_eq!(geometry(&fixture, 1), (0, 0, CANVAS, CANVAS));
    assert_eq!(geometry(&fixture, 2), (CANVAS, 0, CANVAS / 2, CANVAS / 2));
}

#[test]
fn ipc_reports_each_outputs_own_scale() {
    let mut fixture = Fixture::with_outputs(1.5, SECOND_AT_2, &["headless-2"]);
    let Response::Outputs { outputs } = fixture.state.handle_request(Request::Outputs) else {
        panic!("an outputs reply");
    };
    let reported: Vec<(String, f64, i32)> = outputs
        .iter()
        .map(|output| (output.name.clone(), output.scale, output.rect.width))
        .collect();
    // 200 / 1.5 = 133.3.., which Smithay rounds up to 134.
    assert_eq!(
        reported,
        [
            ("headless".to_owned(), 1.5, 134),
            ("headless-2".to_owned(), 2.0, CANVAS / 2),
        ],
        "an agent converting output 2's rect to its screenshot pixels needs 2, not the default"
    );
}

/// An entry for the only output wins over the default for it -- so nothing
/// that means "this output's scale" may read the default, including what a
/// brand-new surface is told.
#[test]
fn an_entry_for_the_only_output_is_what_its_surfaces_hear() {
    let mut fixture =
        Fixture::with_outputs(1.0, "[[outputs]]\nname = \"headless\"\nscale = 2.0\n", &[]);
    assert_eq!(scales(&fixture), [("headless".to_owned(), 2.0, 2)]);
    let Ack::Negotiated {
        preferred_scale,
        preferred_buffer_scale,
        output_scale,
        ..
    } = fixture.run(Step::Negotiate)
    else {
        panic!("the negotiate step must report what it saw");
    };
    assert_eq!(preferred_scale, Some(2.0));
    assert_eq!(preferred_buffer_scale, Some(2));
    assert_eq!(output_scale, Some(2));
}

/// A surface with no role yet is told the pointer's output's scale: that is
/// where a new window opens.
#[test]
fn a_new_surface_hears_the_scale_of_the_output_under_the_pointer() {
    let mut fixture = Fixture::with_outputs(1.0, SECOND_AT_2, &["headless-2"]);
    // Onto the second output: its left edge is at `CANVAS` logical pixels.
    fixture.state.pointer_move(f64::from(CANVAS) + 20.0, 20.0);
    let Ack::Negotiated {
        preferred_scale,
        preferred_buffer_scale,
        ..
    } = fixture.run(Step::Negotiate)
    else {
        panic!("the negotiate step must report what it saw");
    };
    assert_eq!(preferred_scale, Some(2.0));
    assert_eq!(preferred_buffer_scale, Some(2));
}

/// A window carried to an output at another scale is re-told that output's
/// scale -- both halves -- and re-told the first's when it comes back. The
/// straddle rule's everyday case: a window's scale follows the output the
/// core places it on.
#[test]
fn a_window_moved_across_outputs_is_told_each_outputs_scale() {
    let mut fixture = Fixture::with_outputs(1.0, SECOND_AT_2, &["headless-2"]);
    // The pointer starts centred on the first output, so the window opens
    // there.
    fixture.run(Step::MapScaledWindow {
        buffer: 40,
        destination: None,
    });
    // At 1 the integer companion is Smithay's cached default, so nothing is
    // sent for it until it first moves.
    assert_eq!(heard(&mut fixture), (Some(1.0), None));

    assert!(
        fixture
            .state
            .act(Action::MoveFocusedWindowToOutput(OutputId(2)))
    );
    fixture.settle();
    let placed = fixture.state.world.arrange();
    assert_eq!(
        placed.placements.first().map(|placement| placement.output),
        Some(OutputId(2)),
        "the move itself"
    );
    assert_eq!(
        heard(&mut fixture),
        (Some(2.0), Some(2)),
        "a window on the 2x output must be told 2x"
    );

    assert!(
        fixture
            .state
            .act(Action::MoveFocusedWindowToOutput(OutputId(1)))
    );
    fixture.settle();
    assert_eq!(heard(&mut fixture), (Some(1.0), Some(1)), "and back");
}

/// A window that never moves is told nothing more by later `apply()`s --
/// the per-window check is a `Cell` compare, and the cached sends are
/// silent. (With one scale everywhere that is every window, which is why a
/// session without `[[outputs]]` sees exactly the traffic it did before.)
#[test]
fn a_window_that_stays_put_is_told_nothing_more() {
    let mut fixture = Fixture::with_outputs(1.0, SECOND_AT_2, &["headless-2"]);
    fixture.run(Step::MapScaledWindow {
        buffer: 40,
        destination: None,
    });
    let counted = |ack: Ack| match ack {
        Ack::Scales { events, .. } => events,
        _ => panic!("the report step must report what it saw"),
    };
    let before = counted(fixture.run(Step::ReportScales));
    // The fractional value once at map time; the integer companion at 1 is
    // Smithay's cached default, so never sent.
    assert_eq!(before, (1, 0), "told once at map time");
    for _ in 0..5 {
        fixture.state.apply();
        fixture.settle();
    }
    assert_eq!(counted(fixture.run(Step::ReportScales)), before);
}

/// A reload that changes one entry re-scales that output alone: the other
/// keeps its scale and geometry, the reply names the entry's field, and the
/// window on the re-scaled output hears the new value.
#[test]
fn a_reload_rescales_only_the_output_whose_entry_changed() {
    let mut fixture = Fixture::with_outputs(1.0, SECOND_AT_2, &["headless-2"]);
    fixture.install_config(&format!("[output]\nscale = 1.0\n{SECOND_AT_2}"));
    fixture.run(Step::MapScaledWindow {
        buffer: 40,
        destination: None,
    });
    assert!(
        fixture
            .state
            .act(Action::MoveFocusedWindowToOutput(OutputId(2)))
    );
    fixture.settle();
    assert_eq!(heard(&mut fixture), (Some(2.0), Some(2)));

    let response = fixture
        .reload_with("[output]\nscale = 1.0\n[[outputs]]\nname = \"headless-2\"\nscale = 1.5\n");
    let Response::Reloaded { applied, refused } = response else {
        panic!("a valid reload should report, not error: {response:?}");
    };
    assert_eq!(applied, ["outputs.headless-2.scale"]);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(
        scales(&fixture),
        [
            ("headless".to_owned(), 1.0, 1),
            ("headless-2".to_owned(), 1.5, 2),
        ]
    );
    assert_eq!(geometry(&fixture, 1), (0, 0, CANVAS, CANVAS));
    // 200 / 1.5 rounds up to 134.
    assert_eq!(geometry(&fixture, 2), (CANVAS, 0, 134, 134));
    assert_eq!(heard(&mut fixture), (Some(1.5), Some(2)));

    // The same file again changes nothing, so says nothing.
    let response = fixture
        .reload_with("[output]\nscale = 1.0\n[[outputs]]\nname = \"headless-2\"\nscale = 1.5\n");
    let Response::Reloaded { applied, refused } = response else {
        panic!("a valid reload should report, not error: {response:?}");
    };
    assert!(
        applied.is_empty() && refused.is_empty(),
        "{applied:?} {refused:?}"
    );
}

/// The default moves only the outputs without an entry of their own; an
/// output with one keeps its scale through a change to the default.
#[test]
fn a_default_reload_leaves_an_output_with_its_own_entry_alone() {
    let mut fixture = Fixture::with_outputs(1.0, SECOND_AT_2, &["headless-2"]);
    fixture.install_config(SECOND_AT_2);
    let response = fixture.reload_with(&format!("[output]\nscale = 1.25\n{SECOND_AT_2}"));
    let Response::Reloaded { applied, refused } = response else {
        panic!("a valid reload should report, not error: {response:?}");
    };
    assert_eq!(applied, ["output.scale"]);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(
        scales(&fixture),
        [
            ("headless".to_owned(), 1.25, 2),
            ("headless-2".to_owned(), 2.0, 2),
        ]
    );
    // Output 2 moved right with output 1's new logical width (160), and kept
    // its own size.
    assert_eq!(geometry(&fixture, 2), (160, 0, CANVAS / 2, CANVAS / 2));
}

/// An entry for an output that is not connected is stored and reported --
/// it takes effect when that output appears -- but no connected output is
/// re-announced for it.
#[test]
fn a_reload_of_an_absent_outputs_entry_stores_it_and_moves_nothing() {
    let mut fixture = Fixture::with_outputs(1.0, "", &["headless-2"]);
    fixture.install_config("");
    let response = fixture.reload_with("[[outputs]]\nname = \"DP-9\"\nscale = 3\n");
    let Response::Reloaded { applied, refused } = response else {
        panic!("a valid reload should report, not error: {response:?}");
    };
    assert_eq!(applied, ["outputs.DP-9.scale"]);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(
        scales(&fixture),
        [
            ("headless".to_owned(), 1.0, 1),
            ("headless-2".to_owned(), 1.0, 1),
        ]
    );
    assert_eq!(fixture.state.configured_scale("DP-9"), 3.0);
    // ...and an output by that name created later runs at it.
    let id = crate::compositor::headless::add_output(&mut fixture.state, "DP-9", CANVAS, CANVAS)
        .expect("a third output");
    let output = fixture.state.outputs.get(id).expect("the new output");
    assert_eq!(scale_of(output), 3.0);
}

/// A changed `mode` applies live -- the output resizes to it -- beside the
/// scale that applies with it, and a second reload of the same file is
/// silent, since the stored entries agree with it now.
#[test]
fn a_reload_applies_a_changed_mode_by_resizing() {
    let mut fixture = Fixture::with_outputs(1.0, SECOND_AT_2, &["headless-2"]);
    fixture.install_config(SECOND_AT_2);
    let file = "[[outputs]]\nname = \"headless-2\"\nscale = 1.0\nmode = \"100x80\"\n";
    let response = fixture.reload_with(file);
    let Response::Reloaded { applied, refused } = response else {
        panic!("a valid reload should report, not error: {response:?}");
    };
    assert_eq!(
        applied,
        ["outputs.headless-2.scale", "outputs.headless-2.mode"]
    );
    assert!(refused.is_empty(), "{refused:?}");
    let second = fixture.state.outputs.get(OutputId(2)).expect("output 2");
    let mode = second.current_mode().expect("a mode");
    assert_eq!(
        (mode.size.w, mode.size.h),
        (100, 80),
        "the reloaded mode must resize the output"
    );
    assert_eq!(scale_of(second), 1.0);

    // Same file again: stored entries agree, so nothing reports.
    let response = fixture.reload_with(file);
    let Response::Reloaded { applied, refused } = response else {
        panic!("a valid reload should report, not error: {response:?}");
    };
    assert!(applied.is_empty(), "{applied:?}");
    assert!(refused.is_empty(), "{refused:?}");
}

/// Scales stop mixing when the only output at another scale goes away: its
/// window, adopted by the remaining output, is re-told that output's scale
/// even though `apply()` no longer looks (it only re-tells while the outputs
/// disagree) -- `remove_output` re-tells everything on that transition.
#[test]
fn a_window_adopted_when_scales_stop_mixing_is_told_the_adopters_scale() {
    let mut fixture = Fixture::with_outputs(1.0, SECOND_AT_2, &["headless-2"]);
    assert!(fixture.state.mixed_scales);
    fixture.run(Step::MapScaledWindow {
        buffer: 40,
        destination: None,
    });
    assert!(
        fixture
            .state
            .act(Action::MoveFocusedWindowToOutput(OutputId(2)))
    );
    fixture.settle();
    assert_eq!(heard(&mut fixture), (Some(2.0), Some(2)));

    assert!(fixture.state.remove_output(OutputId(2)), "the removal");
    fixture.settle();
    assert!(!fixture.state.mixed_scales, "one output, one scale");
    assert_eq!(
        heard(&mut fixture),
        (Some(1.0), Some(1)),
        "the adopted window must hear the adopting output's scale"
    );
}

/// Scales start mixing when an output at another scale is plugged in: a
/// window carried onto it is told its scale, though every `apply()` before
/// the add skipped the per-window walk.
#[test]
fn a_window_moved_onto_a_hotplugged_output_hears_its_scale() {
    let mut fixture = Fixture::with_outputs(1.0, "[[outputs]]\nname = \"DP-1\"\nscale = 2\n", &[]);
    fixture.run(Step::MapScaledWindow {
        buffer: 40,
        destination: None,
    });
    assert!(!fixture.state.mixed_scales);
    let id = crate::compositor::headless::add_output(&mut fixture.state, "DP-1", CANVAS, CANVAS)
        .expect("a plugged-in output");
    assert!(fixture.state.mixed_scales);
    assert!(fixture.state.act(Action::MoveFocusedWindowToOutput(id)));
    fixture.settle();
    assert_eq!(heard(&mut fixture), (Some(2.0), Some(2)));
}
