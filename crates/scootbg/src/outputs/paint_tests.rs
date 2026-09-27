//! What the model says to draw, and when a waiting reply may go, through a
//! surface's life: configure, resize, clear, close, give up.

use super::{Effect, OutputId, Outputs, Size, Surface};
use crate::color::Color;
use crate::paint::{Drawn, Plan};
use crate::waiters::Progress;

fn size(width: u32, height: u32) -> Size {
    Size { width, height }
}

fn color(text: &str) -> Color {
    Color::parse(text).unwrap()
}

fn drawn(text: &str, width: u32, height: u32, scale: u32) -> Drawn {
    Drawn {
        color: color(text),
        size: size(width, height),
        scale,
    }
}

/// An output settled with a 1600x1000 mode at scale 1, surface pending.
fn pending(outputs: &mut Outputs<()>) -> OutputId {
    let id = outputs.add(1, |_| ());
    let output = &mut outputs.get_mut(id).unwrap().output;
    output.stage_mode(true, 1600, 1000);
    output.stage_name("HEADLESS-1".into());
    output.done();
    assert_eq!(output.settled(), Effect::Create);
    id
}

#[test]
fn nothing_is_drawn_before_the_first_configure() {
    let mut outputs = Outputs::<()>::default();
    let id = outputs.add(1, |_| ());
    let red = Some(color("#c03020"));
    let output = &mut outputs.get_mut(id).unwrap().output;
    assert_eq!(output.plan(red, 1), Plan::Nothing, "waiting");
    assert_eq!(output.progress(red, 1), Progress::Waiting);
    assert_eq!(
        output.progress(None, 1),
        Progress::Done,
        "shows nothing, as wanted"
    );
    output.done();
    let _ = output.settled();
    assert_eq!(output.plan(red, 1), Plan::Nothing, "pending");
    assert_eq!(output.progress(red, 1), Progress::Waiting);
    assert_eq!(output.shows(), None);
}

#[test]
fn a_configured_surface_is_drawn_at_its_size_then_left_alone() {
    let mut outputs = Outputs::<()>::default();
    let id = pending(&mut outputs);
    let red = Some(color("#c03020"));
    let output = &mut outputs.get_mut(id).unwrap().output;
    let _ = output.configure(3, 1600, 1000);
    assert_eq!(output.progress(None, 1), Progress::Done);
    let target = drawn("#c03020", 1600, 1000, 1);
    assert_eq!(output.plan(red, 1), Plan::Show(target));
    assert_eq!(output.progress(red, 1), Progress::Waiting);
    output.drew(target);
    assert_eq!(output.plan(red, 1), Plan::Nothing);
    assert_eq!(output.progress(red, 1), Progress::Done);
    assert_eq!(output.shows(), Some(color("#c03020")));
    // Another color: drawn again.
    let blue = Some(color("#101014"));
    assert_eq!(
        output.plan(blue, 1),
        Plan::Show(drawn("#101014", 1600, 1000, 1))
    );
    assert_eq!(output.progress(blue, 1), Progress::Waiting);
}

/// A later `configure` keeps what is shown (the surface is still mapped)
/// and asks for a redraw at the new size; the scale is part of it too.
#[test]
fn a_resize_or_a_new_scale_redraws() {
    let mut outputs = Outputs::<()>::default();
    let id = pending(&mut outputs);
    let red = Some(color("#c03020"));
    let output = &mut outputs.get_mut(id).unwrap().output;
    let _ = output.configure(3, 1600, 1000);
    output.drew(drawn("#c03020", 1600, 1000, 1));
    assert_eq!(output.configure(4, 800, 500), Effect::Ack(4));
    assert_eq!(output.shows(), Some(color("#c03020")), "still mapped");
    assert_eq!(
        output.plan(red, 1),
        Plan::Show(drawn("#c03020", 800, 500, 1))
    );
    assert_eq!(output.progress(red, 1), Progress::Waiting);
    output.drew(drawn("#c03020", 800, 500, 1));
    assert_eq!(output.plan(red, 1), Plan::Nothing);
    // The same size at another buffer scale (a full-size buffer's path).
    assert_eq!(
        output.plan(red, 2),
        Plan::Show(drawn("#c03020", 800, 500, 2))
    );
}

#[test]
fn clearing_a_drawn_surface_replaces_it_and_waits_for_nothing() {
    let mut outputs = Outputs::<()>::default();
    let id = pending(&mut outputs);
    let output = &mut outputs.get_mut(id).unwrap().output;
    let _ = output.configure(3, 1600, 1000);
    // Nothing drawn: nothing to clear.
    assert_eq!(output.plan(None, 1), Plan::Nothing);
    output.drew(drawn("#c03020", 1600, 1000, 1));
    assert_eq!(output.plan(None, 1), Plan::Clear);
    assert_eq!(output.progress(None, 1), Progress::Waiting);
    output.recreated();
    assert_eq!(output.surface(), Surface::Pending);
    assert_eq!(output.shows(), None);
    assert_eq!(output.progress(None, 1), Progress::Done);
    // Set again: waits for the fresh surface's configure, then draws.
    let red = Some(color("#c03020"));
    assert_eq!(output.progress(red, 1), Progress::Waiting);
    let _ = output.configure(4, 1600, 1000);
    assert_eq!(
        output.plan(red, 1),
        Plan::Show(drawn("#c03020", 1600, 1000, 1)),
        "the fresh surface starts with nothing drawn"
    );
}

#[test]
fn a_closed_surface_forgets_what_it_drew() {
    let mut outputs = Outputs::<()>::default();
    let id = pending(&mut outputs);
    let red = Some(color("#c03020"));
    let output = &mut outputs.get_mut(id).unwrap().output;
    let _ = output.configure(3, 1600, 1000);
    output.drew(drawn("#c03020", 1600, 1000, 1));
    assert_eq!(output.closed(), Effect::DestroyAndRetry);
    assert_eq!(output.shows(), None);
    assert_eq!(output.progress(red, 1), Progress::Waiting);
    assert_eq!(output.retry(), Effect::Create);
    let _ = output.configure(4, 1600, 1000);
    assert_eq!(
        output.plan(red, 1),
        Plan::Show(drawn("#c03020", 1600, 1000, 1))
    );
    // Closed again: given up, and nothing waits on it any more.
    assert_eq!(output.closed(), Effect::DestroyAndGiveUp);
    assert_eq!(output.progress(red, 1), Progress::Done);
    assert_eq!(output.plan(red, 1), Plan::Nothing);
    // `recreated` only acts on a live surface.
    output.recreated();
    assert_eq!(output.surface(), Surface::GaveUp);
}

#[test]
fn a_failed_draw_is_not_retried_until_something_changes() {
    let mut outputs = Outputs::<()>::default();
    let id = pending(&mut outputs);
    let red = Some(color("#c03020"));
    let output = &mut outputs.get_mut(id).unwrap().output;
    let _ = output.configure(3, 1600, 1000);
    output.draw_failed();
    assert_eq!(output.plan(red, 1), Plan::Nothing, "no retry loop");
    assert_eq!(output.progress(red, 1), Progress::Failed);
    // A new request is a new chance.
    output.want(7);
    assert_eq!(output.stamp(), 7);
    assert_eq!(output.progress(red, 1), Progress::Waiting);
    output.draw_failed();
    // So is a new configure.
    let _ = output.configure(4, 800, 500);
    assert_eq!(
        output.plan(red, 1),
        Plan::Show(drawn("#c03020", 800, 500, 1))
    );
}

/// A zero `configure` before any mode is known cannot be sized: nothing
/// is drawn and the reply waits, until a mode arrives.
#[test]
fn an_unsized_surface_waits_for_a_mode() {
    let mut outputs = Outputs::<()>::default();
    let id = outputs.add(1, |_| ());
    let red = Some(color("#c03020"));
    let output = &mut outputs.get_mut(id).unwrap().output;
    let _ = output.settled();
    let _ = output.configure(1, 0, 0);
    assert_eq!(output.plan(red, 1), Plan::Nothing);
    assert_eq!(output.progress(red, 1), Progress::Waiting);
    output.stage_mode(true, 1280, 720);
    output.done();
    assert_eq!(
        output.plan(red, 1),
        Plan::Show(drawn("#c03020", 1280, 720, 1))
    );
}

#[test]
fn stamps_start_at_zero() {
    let mut outputs = Outputs::<()>::default();
    let id = pending(&mut outputs);
    assert_eq!(outputs.get_mut(id).unwrap().output.stamp(), 0);
}
