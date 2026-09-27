use super::*;
use crate::{Action, Horizontal, Size, SizeHints};

fn observe(world: &mut World, id: u64, requested: Size, actual: Size) -> bool {
    world.handle_event(Event::FrameObserved {
        id: WindowId(id),
        requested,
        actual,
    })
}

#[test]
fn a_window_that_refuses_to_shrink_widens_its_column() {
    let mut world = world();
    open(&mut world, 1);
    observe(&mut world, 1, Size::new(485, 580), Size::new(600, 580));
    assert_eq!(placement(&world, 1).rect.w, 600);
}

#[test]
fn ending_up_smaller_or_within_tolerance_teaches_nothing() {
    let mut world = world();
    open(&mut world, 1);
    observe(&mut world, 1, Size::new(485, 580), Size::new(300, 580));
    observe(&mut world, 1, Size::new(485, 580), Size::new(487, 582));
    assert_eq!(placement(&world, 1).rect, Rect::new(10, 10, 485, 580));
}

#[test]
fn a_late_answer_to_an_older_request_teaches_nothing() {
    let mut world = world();
    open(&mut world, 1);
    world.handle_action(Action::CycleColumnWidth);
    world.handle_action(Action::CycleColumnWidth);
    // The app's reply to the earlier full-width request arrives late.
    observe(&mut world, 1, Size::new(980, 580), Size::new(980, 580));
    assert_eq!(placement(&world, 1).rect.w, 485);
}

#[test]
fn learned_minimums_are_capped_to_the_output() {
    let mut world = world();
    open(&mut world, 1);
    observe(&mut world, 1, Size::new(485, 580), Size::new(5000, 5000));
    assert_eq!(placement(&world, 1).rect, Rect::new(10, 10, 980, 580));
}

#[test]
fn choosing_a_width_forgets_learned_widths() {
    let mut world = world();
    open(&mut world, 1);
    observe(&mut world, 1, Size::new(485, 580), Size::new(600, 580));
    world.handle_action(Action::CycleColumnWidth);
    world.handle_action(Action::CycleColumnWidth);
    assert_eq!(placement(&world, 1).rect.w, 485);
}

#[test]
fn taller_than_asked_raises_the_minimum_height() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(Action::ConsumeOrExpel(Horizontal::Left));
    observe(&mut world, 1, Size::new(485, 285), Size::new(485, 400));
    assert_eq!(placement(&world, 1).rect.h, 400);
    assert_eq!(placement(&world, 2).rect, Rect::new(10, 420, 485, 170));
}
#[test]
fn window_changed_updates_size_hints() {
    let mut world = world();
    open(&mut world, 1);
    let wide = WindowInfo {
        hints: SizeHints {
            min: Size::new(700, 0),
            ..SizeHints::default()
        },
        ..WindowInfo::default()
    };
    world.handle_event(Event::WindowChanged {
        id: WindowId(1),
        info: wide,
    });
    assert_eq!(placement(&world, 1).rect.w, 700);
}

#[test]
fn a_frame_reports_whether_it_changed_the_arrangement() {
    // What the shell's flush applies on: a refusal that raised the minimum
    // answers true, everything that changed nothing answers false -- so a
    // video committing at frame rate never earns a relayout.
    let mut world = world();
    open(&mut world, 1);
    assert!(observe(
        &mut world,
        1,
        Size::new(485, 580),
        Size::new(600, 580)
    ));
    // Same refusal again: the minimum is already there, nothing moved.
    assert!(!observe(
        &mut world,
        1,
        Size::new(600, 580),
        Size::new(600, 580)
    ));
    // Smaller, or within tolerance: teaches nothing.
    assert!(!observe(
        &mut world,
        1,
        Size::new(600, 580),
        Size::new(300, 580)
    ));
    assert!(!observe(
        &mut world,
        1,
        Size::new(600, 580),
        Size::new(602, 582)
    ));
    // Unknown windows change nothing.
    assert!(!observe(
        &mut world,
        9,
        Size::new(485, 580),
        Size::new(600, 580)
    ));
}

#[test]
fn a_fullscreen_frame_reports_no_change() {
    let mut world = world();
    open(&mut world, 1);
    world.handle_event(Event::FullscreenRequested {
        id: WindowId(1),
        fullscreen: true,
    });
    // An output-sized frame says nothing about the tiled minimum.
    assert!(!observe(
        &mut world,
        1,
        Size::new(485, 580),
        Size::new(980, 580)
    ));
}

#[test]
fn a_floating_frame_reports_only_a_move() {
    use crate::Event::FloatingRequested;
    let mut world = world();
    open(&mut world, 1);
    world.handle_event(FloatingRequested {
        id: WindowId(1),
        floating: true,
        size: None,
    });
    // First frame places it: the arrangement moved.
    assert!(observe(&mut world, 1, Size::default(), Size::new(400, 300)));
    // Same size again: already there.
    assert!(!observe(
        &mut world,
        1,
        Size::default(),
        Size::new(400, 300)
    ));
    // A new size moves it again -- and never teaches the strip.
    assert!(observe(&mut world, 1, Size::default(), Size::new(500, 300)));
    assert_eq!(placement(&world, 1).rect.w, 500);
}

#[test]
fn only_a_frame_observation_can_report_change() {
    // Every other event answers false: their callers already re-apply
    // unconditionally, so the return is only meaningful for frames.
    let mut world = world();
    open(&mut world, 1);
    assert!(!world.handle_event(Event::FocusObserved { id: WindowId(1) }));
    assert!(!world.handle_event(Event::WindowClosed { id: WindowId(1) }));
}
