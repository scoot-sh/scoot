use super::{Effect, Frame, Output, OutputId, Outputs, Plan, Rotation, Size, Surface};
use crate::bar::{Bar, Margin};
use crate::density::Scale;

fn size(width: u32, height: u32) -> Size {
    Size { width, height }
}

fn frame(width: u32, height: u32, scale: Scale) -> Frame {
    Frame {
        size: size(width, height),
        scale,
    }
}

/// One output, bound as global 7, with a 1920×1080 mode at scale 1.
fn bound() -> (Outputs<()>, OutputId) {
    let mut outputs = Outputs::default();
    let id = outputs.add(7, |_| ());
    let output = &mut outputs.get_mut(id).unwrap().output;
    output.stage_mode(true, 1920, 1080);
    output.stage_scale(1);
    output.done();
    (outputs, id)
}

fn output(outputs: &mut Outputs<()>, id: OutputId) -> &mut Output {
    &mut outputs.get_mut(id).unwrap().output
}

/// Bound, settled and configured at `width` × `height`.
fn configured(width: u32, height: u32) -> (Outputs<()>, OutputId) {
    let (mut outputs, id) = bound();
    let output = output(&mut outputs, id);
    assert_eq!(output.settled(), Effect::Create);
    assert_eq!(output.configure(5, width, height), Effect::Ack(5));
    (outputs, id)
}

#[test]
fn a_bound_output_waits_for_its_settle_then_creates_once() {
    let (mut outputs, id) = bound();
    let output = output(&mut outputs, id);
    assert_eq!(output.surface(), Surface::Waiting);
    assert_eq!(output.plan(&Bar::default()), Plan::Nothing);
    assert_eq!(output.settled(), Effect::Create);
    assert_eq!(output.surface(), Surface::Pending);
    // A second callback (cannot happen, but) creates nothing more.
    assert_eq!(output.settled(), Effect::None);
    // Nothing to draw before the first configure.
    assert_eq!(output.plan(&Bar::default()), Plan::Nothing);
}

#[test]
fn properties_apply_at_done_and_without_one_at_the_settle() {
    let mut outputs: Outputs<()> = Outputs::default();
    let id = outputs.add(1, |_| ());
    let output = output(&mut outputs, id);
    output.stage_name("DP-1".into());
    output.stage_mode(true, 2560, 1440);
    output.stage_scale(2);
    assert_eq!(output.info().name, None);
    assert_eq!(output.info().scale, 1);
    // A v1 output never sends `done`: the settle applies what came.
    assert_eq!(output.settled(), Effect::Create);
    assert_eq!(output.info().name.as_deref(), Some("DP-1"));
    assert_eq!(output.info().mode, Some(size(2560, 1440)));
    assert_eq!(output.info().scale, 2);
    assert_eq!(output.label().to_string(), "output \"DP-1\"");
}

#[test]
fn broken_properties_are_ignored() {
    let (mut outputs, id) = bound();
    let output = output(&mut outputs, id);
    output.stage_mode(true, 0, 1080);
    output.stage_mode(true, -1, 1080);
    output.stage_mode(false, 800, 600);
    output.stage_scale(0);
    output.stage_scale(-3);
    output.done();
    assert_eq!(output.info().mode, Some(size(1920, 1080)));
    assert_eq!(output.info().scale, 1);
}

#[test]
fn a_configure_is_acked_and_then_drawn() {
    let (mut outputs, id) = configured(1920, 28);
    let bar = Bar::default();
    let output = output(&mut outputs, id);
    let want = frame(1920, 28, Scale::Integer(1));
    assert_eq!(output.plan(&bar), Plan::Draw(want));
    output.drew(want);
    // Drawn: nothing more until something changes. No wakeups at idle.
    assert_eq!(output.plan(&bar), Plan::Nothing);
}

#[test]
fn a_configure_that_changes_nothing_is_committed_not_redrawn() {
    let (mut outputs, id) = configured(1920, 28);
    let bar = Bar::default();
    let output = output(&mut outputs, id);
    let want = frame(1920, 28, Scale::Integer(1));
    output.drew(want);
    assert_eq!(output.configure(6, 1920, 28), Effect::Ack(6));
    assert_eq!(output.plan(&bar), Plan::Commit);
    output.committed();
    assert_eq!(output.plan(&bar), Plan::Nothing);
}

#[test]
fn a_new_size_redraws() {
    let (mut outputs, id) = configured(1920, 28);
    let bar = Bar::default();
    let output = output(&mut outputs, id);
    output.drew(frame(1920, 28, Scale::Integer(1)));
    assert_eq!(output.configure(6, 1280, 28), Effect::Ack(6));
    assert_eq!(
        output.plan(&bar),
        Plan::Draw(frame(1280, 28, Scale::Integer(1)))
    );
}

#[test]
fn a_zero_height_is_the_bars_and_a_zero_width_the_outputs_less_the_margins() {
    let (mut outputs, id) = configured(0, 0);
    let bar = Bar {
        height: 30,
        margin: Margin {
            top: 4,
            right: 10,
            bottom: 0,
            left: 10,
        },
        ..Bar::default()
    };
    let output = output(&mut outputs, id);
    assert_eq!(output.surface_size(&bar), Some(size(1900, 30)));
    // Rotated: the logical width is the mode's height.
    output.stage_rotation(Rotation::Sideways);
    output.done();
    assert_eq!(output.surface_size(&bar), Some(size(1060, 30)));
    // At scale 2, half of that.
    output.stage_scale(2);
    output.done();
    assert_eq!(output.surface_size(&bar), Some(size(520, 30)));
}

#[test]
fn a_zero_width_with_no_mode_waits() {
    let mut outputs: Outputs<()> = Outputs::default();
    let id = outputs.add(1, |_| ());
    let output = output(&mut outputs, id);
    assert_eq!(output.settled(), Effect::Create);
    assert_eq!(output.configure(1, 0, 28), Effect::Ack(1));
    assert_eq!(output.surface_size(&Bar::default()), None);
    assert_eq!(output.plan(&Bar::default()), Plan::Nothing);
    // The mode arrives: now it can be drawn.
    output.stage_mode(true, 800, 600);
    output.done();
    assert_eq!(
        output.plan(&Bar::default()),
        Plan::Draw(frame(800, 28, Scale::Integer(1)))
    );
}

#[test]
fn a_scale_change_redraws_at_the_new_scale() {
    let (mut outputs, id) = configured(1920, 28);
    let bar = Bar::default();
    let output = output(&mut outputs, id);
    output.drew(frame(1920, 28, Scale::Integer(1)));
    output.stage_scale(2);
    output.done();
    assert_eq!(
        output.plan(&bar),
        Plan::Draw(frame(1920, 28, Scale::Integer(2)))
    );
    output.drew(frame(1920, 28, Scale::Integer(2)));
    output.prefer_fractional(180);
    assert_eq!(
        output.plan(&bar),
        Plan::Draw(frame(1920, 28, Scale::Fractional(180)))
    );
    output.drew(frame(1920, 28, Scale::Fractional(180)));
    // The same scale again changes nothing.
    output.prefer_fractional(180);
    assert_eq!(output.plan(&bar), Plan::Nothing);
    // A surface integer scale below the fraction's does not win.
    output.prefer_buffer_scale(1);
    assert_eq!(output.plan(&bar), Plan::Nothing);
}

#[test]
fn a_failed_draw_is_not_retried_until_the_frame_changes() {
    let (mut outputs, id) = configured(1920, 28);
    let bar = Bar::default();
    let output = output(&mut outputs, id);
    let want = frame(1920, 28, Scale::Integer(1));
    output.draw_failed(want);
    // Never drawn, so there is nothing to commit either.
    assert_eq!(output.plan(&bar), Plan::Nothing);
    // A new configure is a new chance.
    assert_eq!(output.configure(6, 1920, 28), Effect::Ack(6));
    assert_eq!(output.plan(&bar), Plan::Draw(want));
    output.draw_failed(want);
    // So is a new scale.
    output.stage_scale(2);
    output.done();
    assert_eq!(
        output.plan(&bar),
        Plan::Draw(frame(1920, 28, Scale::Integer(2)))
    );
}

#[test]
fn a_failed_redraw_still_commits_the_ack_on_a_mapped_surface() {
    let (mut outputs, id) = configured(1920, 28);
    let bar = Bar::default();
    let output = output(&mut outputs, id);
    output.drew(frame(1920, 28, Scale::Integer(1)));
    assert_eq!(output.configure(6, 3000, 28), Effect::Ack(6));
    let want = frame(3000, 28, Scale::Integer(1));
    assert_eq!(output.plan(&bar), Plan::Draw(want));
    output.draw_failed(want);
    assert_eq!(output.plan(&bar), Plan::Commit);
    output.committed();
    assert_eq!(output.plan(&bar), Plan::Nothing);
}

#[test]
fn closed_retries_once_then_gives_up() {
    let (mut outputs, id) = configured(1920, 28);
    let bar = Bar::default();
    let output = output(&mut outputs, id);
    output.drew(frame(1920, 28, Scale::Integer(1)));
    assert_eq!(output.closed(), Effect::DestroyAndRetry);
    assert_eq!(output.surface(), Surface::Closed);
    assert_eq!(output.plan(&bar), Plan::Nothing);
    // A second `closed` for the destroyed surface is stale.
    assert_eq!(output.closed(), Effect::None);
    assert_eq!(output.retry(), Effect::Create);
    assert_eq!(output.retry(), Effect::None);
    // The new surface draws from scratch once configured.
    assert_eq!(output.configure(9, 1920, 28), Effect::Ack(9));
    assert_eq!(
        output.plan(&bar),
        Plan::Draw(frame(1920, 28, Scale::Integer(1)))
    );
    assert_eq!(output.closed(), Effect::DestroyAndGiveUp);
    assert_eq!(output.surface(), Surface::GaveUp);
    assert_eq!(output.retry(), Effect::None);
    assert_eq!(output.configure(10, 1920, 28), Effect::None);
    assert_eq!(output.plan(&bar), Plan::Nothing);
}

#[test]
fn a_configure_for_no_live_surface_is_not_acked() {
    let (mut outputs, id) = bound();
    let output = output(&mut outputs, id);
    // Before the surface exists.
    assert_eq!(output.configure(1, 1920, 28), Effect::None);
    assert_eq!(output.surface(), Surface::Waiting);
}

#[test]
fn a_draw_recorded_after_close_does_not_resurrect_it() {
    let (mut outputs, id) = configured(1920, 28);
    let output = output(&mut outputs, id);
    assert_eq!(output.closed(), Effect::DestroyAndRetry);
    output.drew(frame(1920, 28, Scale::Integer(1)));
    assert_eq!(output.retry(), Effect::Create);
    assert_eq!(output.configure(2, 1920, 28), Effect::Ack(2));
    assert!(matches!(output.plan(&Bar::default()), Plan::Draw(_)));
}

#[test]
fn outputs_come_and_go_with_ids_never_reused() {
    let mut outputs: Outputs<()> = Outputs::default();
    let a = outputs.add(1, |_| ());
    let b = outputs.add(2, |_| ());
    assert_ne!(a, b);
    assert_eq!(outputs.len(), 2);
    assert!(outputs.remove_global(1).is_some());
    assert!(outputs.remove_global(1).is_none());
    assert!(outputs.get_mut(a).is_none());
    // The compositor hands the replugged monitor its old global name.
    let c = outputs.add(1, |_| ());
    assert_ne!(c, a);
    assert!(outputs.get_mut(a).is_none());
    assert!(outputs.get_mut(c).is_some());
    // Removing an unknown global (another interface's) is not an output.
    assert!(outputs.remove_global(99).is_none());
    assert_eq!(outputs.len(), 2);
}

#[test]
fn zero_outputs_is_a_normal_state() {
    let mut outputs: Outputs<()> = Outputs::default();
    assert_eq!(outputs.iter_mut().count(), 0);
    let a = outputs.add(1, |_| ());
    assert!(outputs.remove_global(1).is_some());
    assert_eq!(outputs.len(), 0);
    assert!(outputs.get_mut(a).is_none());
}

#[test]
fn an_escaped_name_labels_the_output() {
    let (mut outputs, id) = bound();
    let output = output(&mut outputs, id);
    assert_eq!(output.label().to_string(), "an unnamed output");
    output.stage_name("evil\u{1b}[2J\"".into());
    output.done();
    assert_eq!(output.label().to_string(), "output \"evil\\u{1b}[2J\\\"\"");
}
