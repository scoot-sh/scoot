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
    assert_eq!(output.plan(&Bar::default(), false), Plan::Nothing);
    assert_eq!(output.settled(), Effect::Create);
    assert_eq!(output.surface(), Surface::Pending);
    // A second callback (cannot happen, but) creates nothing more.
    assert_eq!(output.settled(), Effect::None);
    // Nothing to draw before the first configure.
    assert_eq!(output.plan(&Bar::default(), false), Plan::Nothing);
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
    assert_eq!(output.plan(&bar, false), Plan::Draw(want));
    output.drew(want);
    // Drawn: nothing more until something changes. No wakeups at idle.
    assert_eq!(output.plan(&bar, false), Plan::Nothing);
}

#[test]
fn a_configure_that_changes_nothing_is_committed_not_redrawn() {
    let (mut outputs, id) = configured(1920, 28);
    let bar = Bar::default();
    let output = output(&mut outputs, id);
    let want = frame(1920, 28, Scale::Integer(1));
    output.drew(want);
    assert_eq!(output.configure(6, 1920, 28), Effect::Ack(6));
    assert_eq!(output.plan(&bar, false), Plan::Commit);
    output.committed();
    assert_eq!(output.plan(&bar, false), Plan::Nothing);
}

#[test]
fn a_new_size_redraws() {
    let (mut outputs, id) = configured(1920, 28);
    let bar = Bar::default();
    let output = output(&mut outputs, id);
    output.drew(frame(1920, 28, Scale::Integer(1)));
    assert_eq!(output.configure(6, 1280, 28), Effect::Ack(6));
    assert_eq!(
        output.plan(&bar, false),
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
    assert_eq!(output.plan(&Bar::default(), false), Plan::Nothing);
    // The mode arrives: now it can be drawn.
    output.stage_mode(true, 800, 600);
    output.done();
    assert_eq!(
        output.plan(&Bar::default(), false),
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
        output.plan(&bar, false),
        Plan::Draw(frame(1920, 28, Scale::Integer(2)))
    );
    output.drew(frame(1920, 28, Scale::Integer(2)));
    output.prefer_fractional(180);
    assert_eq!(
        output.plan(&bar, false),
        Plan::Draw(frame(1920, 28, Scale::Fractional(180)))
    );
    output.drew(frame(1920, 28, Scale::Fractional(180)));
    // The same scale again changes nothing.
    output.prefer_fractional(180);
    assert_eq!(output.plan(&bar, false), Plan::Nothing);
    // A surface integer scale below the fraction's does not win.
    output.prefer_buffer_scale(1);
    assert_eq!(output.plan(&bar, false), Plan::Nothing);
}

#[test]
fn a_failed_draw_is_retried_a_few_times_then_quiet_until_the_frame_changes() {
    let (mut outputs, id) = configured(1920, 28);
    let bar = Bar::default();
    let output = output(&mut outputs, id);
    let want = frame(1920, 28, Scale::Integer(1));
    // The first failure is said; the retries stay quiet.
    assert!(output.draw_failed(want));
    // A later turn tries the same frame again: a clock tick that changed
    // the text (`stale`), or anything else (never shown). The clock
    // recovers as soon as a retry goes through.
    assert_eq!(output.plan(&bar, true), Plan::Draw(want));
    assert!(!output.draw_failed(want));
    assert_eq!(output.plan(&bar, false), Plan::Draw(want));
    assert!(!output.draw_failed(want));
    // Past the bound (three retries, mirroring scoot's present-skip
    // retry): quiet, even for a tick that changed the text. Capping the
    // bound at 0 makes the `Draw` assertions above fail.
    assert!(!output.draw_failed(want));
    assert_eq!(output.plan(&bar, true), Plan::Nothing);
    assert_eq!(output.plan(&bar, false), Plan::Nothing);
    // A new configure is a new chance, with its own streak.
    assert_eq!(output.configure(6, 1920, 28), Effect::Ack(6));
    assert_eq!(output.plan(&bar, false), Plan::Draw(want));
    assert!(output.draw_failed(want));
    // So is a new scale.
    output.stage_scale(2);
    output.done();
    assert_eq!(
        output.plan(&bar, false),
        Plan::Draw(frame(1920, 28, Scale::Integer(2)))
    );
}

#[test]
fn a_shown_draw_after_failures_recovers_the_clock() {
    let (mut outputs, id) = configured(1920, 28);
    let bar = Bar::default();
    let output = output(&mut outputs, id);
    let want = frame(1920, 28, Scale::Integer(1));
    // Tick one's draw refused under fd pressure; tick two's retry shows.
    assert!(output.draw_failed(want));
    assert_eq!(output.plan(&bar, true), Plan::Draw(want));
    output.drew(want);
    // Drawn: nothing more until something changes. No frozen clock.
    assert_eq!(output.plan(&bar, false), Plan::Nothing);
    assert_eq!(output.plan(&bar, true), Plan::Draw(want));
}

#[test]
fn a_reload_redraws_once_with_no_modules_placed() {
    let (mut outputs, id) = configured(1920, 28);
    let bar = Bar::default();
    let output = output(&mut outputs, id);
    let want = frame(1920, 28, Scale::Integer(1));
    output.drew(want);
    // Drawn: nothing more until something changes. With no modules
    // placed `stale` is always false (no revisions to compare), yet a
    // reload must still draw once, or the removed modules' pixels stay.
    assert_eq!(output.plan(&bar, false), Plan::Nothing);
    output.invalidate();
    assert_eq!(output.plan(&bar, false), Plan::Draw(want));
    output.drew(want);
    assert_eq!(output.plan(&bar, false), Plan::Nothing);
}

#[test]
fn a_reload_is_a_new_chance_after_a_failed_draw() {
    let (mut outputs, id) = configured(1920, 28);
    let bar = Bar::default();
    let output = output(&mut outputs, id);
    let want = frame(1920, 28, Scale::Integer(1));
    for _ in 0..4 {
        output.draw_failed(want);
    }
    assert_eq!(output.plan(&bar, true), Plan::Nothing);
    output.invalidate();
    assert_eq!(output.plan(&bar, false), Plan::Draw(want));
}

#[test]
fn a_failed_redraw_retries_first_then_commits_the_ack_once_quiet() {
    let (mut outputs, id) = configured(1920, 28);
    let bar = Bar::default();
    let output = output(&mut outputs, id);
    output.drew(frame(1920, 28, Scale::Integer(1)));
    assert_eq!(output.configure(6, 3000, 28), Effect::Ack(6));
    let want = frame(3000, 28, Scale::Integer(1));
    assert_eq!(output.plan(&bar, false), Plan::Draw(want));
    // The first failures retry the new frame; past the bound it goes
    // quiet, but the acked `configure` still needs its commit on the
    // mapped surface.
    assert!(output.draw_failed(want));
    assert_eq!(output.plan(&bar, false), Plan::Draw(want));
    for _ in 0..3 {
        output.draw_failed(want);
    }
    assert_eq!(output.plan(&bar, false), Plan::Commit);
    output.committed();
    assert_eq!(output.plan(&bar, false), Plan::Nothing);
}

#[test]
fn closed_retries_once_then_gives_up() {
    let (mut outputs, id) = configured(1920, 28);
    let bar = Bar::default();
    let output = output(&mut outputs, id);
    output.drew(frame(1920, 28, Scale::Integer(1)));
    assert_eq!(output.closed(), Effect::DestroyAndRetry);
    assert_eq!(output.surface(), Surface::Closed);
    assert_eq!(output.plan(&bar, false), Plan::Nothing);
    // A second `closed` for the destroyed surface is stale.
    assert_eq!(output.closed(), Effect::None);
    assert_eq!(output.retry(), Effect::Create);
    assert_eq!(output.retry(), Effect::None);
    // The new surface draws from scratch once configured.
    assert_eq!(output.configure(9, 1920, 28), Effect::Ack(9));
    assert_eq!(
        output.plan(&bar, false),
        Plan::Draw(frame(1920, 28, Scale::Integer(1)))
    );
    assert_eq!(output.closed(), Effect::DestroyAndGiveUp);
    assert_eq!(output.surface(), Surface::GaveUp);
    assert_eq!(output.retry(), Effect::None);
    assert_eq!(output.configure(10, 1920, 28), Effect::None);
    assert_eq!(output.plan(&bar, false), Plan::Nothing);
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
    assert!(matches!(output.plan(&Bar::default(), false), Plan::Draw(_)));
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

#[test]
fn a_negative_size_sent_as_a_uint_is_taken_as_zero() {
    // sway 1.12, 1280 wide, `--margin 0,1024`: -768 as a `uint`.
    let (mut outputs, id) = configured(4_294_966_528, 28);
    let bar = Bar {
        margin: Margin {
            top: 0,
            right: 1024,
            bottom: 0,
            left: 1024,
        },
        ..Bar::default()
    };
    let output = output(&mut outputs, id);
    // Resolved as 0: the output (1920) less the margins, at least 1.
    assert_eq!(output.surface_size(&bar), Some(size(1, 28)));
    // Either side, and the boundary itself.
    assert_eq!(output.configure(6, 1920, u32::MAX), Effect::Ack(6));
    assert_eq!(output.surface_size(&bar), Some(size(1920, 28)));
    assert_eq!(output.configure(7, i32::MAX as u32, 28), Effect::Ack(7));
    assert_eq!(output.surface_size(&bar), Some(size(i32::MAX as u32, 28)));
    assert_eq!(output.configure(8, 1 << 31, 28), Effect::Ack(8));
    assert_eq!(output.surface_size(&bar), Some(size(1, 28)));
}
