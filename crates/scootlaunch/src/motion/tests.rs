//! The motion's tests, on a made-up clock: no sleeping, no real time.

use std::time::Duration;

use scootui::motion::{Curve, Timing};

use super::*;

fn at(base: Instant, ms: u64) -> Instant {
    base + Duration::from_millis(ms)
}

fn ease() -> Motion {
    Motion::new(Timing::new(Curve::EaseOut, 200, 0.0))
}

fn spring() -> Motion {
    Motion::new(Timing::new(Curve::Spring, 200, 1.0))
}

#[test]
fn no_motion_is_always_at_rest_and_never_animates() {
    let base = Instant::now();
    let mut motion = Motion::new(Timing::NONE);
    assert!(!motion.active());
    motion.start(base);
    for ms in [0, 1, 100, 10_000] {
        assert_eq!(motion.pose(at(base, ms)), Pose::REST);
        assert!(!motion.animating(at(base, ms)));
    }
    // Nothing to play on the way out: the caller exits at once.
    assert!(!motion.begin_exit(base));
    assert!(!motion.exit_done(base));
    // The highlight snaps whatever it is told.
    motion.set_highlight(Some(0.0), true, base);
    motion.set_highlight(Some(40.0), true, base);
    assert_eq!(motion.highlight_at(at(base, 1)), Some(40.0));
    assert!(!motion.animating(base));
}

#[test]
fn a_zero_duration_is_no_motion() {
    let motion = Motion::new(Timing::new(Curve::Spring, 0, 1.0));
    assert!(!motion.active());
}

#[test]
fn the_entrance_grows_and_fades_in_and_lands_exactly_at_rest() {
    let base = Instant::now();
    let mut motion = ease();
    // Before the first frame there is nothing to animate.
    assert_eq!(motion.pose(base), Pose::REST);
    assert!(!motion.animating(base));
    motion.start(base);
    let first = motion.pose(base);
    assert_eq!(first.alpha, 0);
    assert!((first.scale - ENTER_SCALE).abs() < 1e-6, "{first:?}");
    let mut last = first;
    for ms in (10..200).step_by(10) {
        let pose = motion.pose(at(base, ms));
        assert!(pose.alpha >= last.alpha, "{ms} ms: alpha went back");
        assert!(pose.scale >= last.scale - 1e-6, "{ms} ms: scale went back");
        assert!(motion.animating(at(base, ms)), "{ms} ms");
        last = pose;
    }
    assert_eq!(motion.pose(at(base, 200)), Pose::REST);
    assert!(!motion.animating(at(base, 200)));
    assert!(!motion.animating(at(base, 100_000)));
}

#[test]
fn a_second_start_does_not_restart_the_entrance() {
    let base = Instant::now();
    let mut motion = ease();
    motion.start(base);
    let mid = motion.pose(at(base, 100));
    motion.start(at(base, 100));
    assert_eq!(motion.pose(at(base, 100)), mid);
}

#[test]
fn a_spring_may_overshoot_the_scale_but_never_the_alpha_or_the_cap() {
    let base = Instant::now();
    let mut motion = spring();
    motion.start(base);
    let mut passed_one = false;
    for ms in 0..=200 {
        let pose = motion.pose(at(base, ms));
        passed_one |= pose.scale > 1.0;
        assert!(pose.scale <= MAX_SCALE && pose.scale >= 0.5, "{pose:?}");
    }
    assert!(passed_one, "the spring bounces past its size");
    assert_eq!(motion.pose(at(base, 200)), Pose::REST);
}

#[test]
fn a_dismissal_fades_to_nothing_and_never_overshoots() {
    let base = Instant::now();
    let mut motion = spring();
    motion.start(base);
    let rest = at(base, 500);
    assert!(motion.begin_exit(rest));
    assert_eq!(motion.pose(rest), Pose::REST);
    let mut last = 255u8;
    for ms in 0..=200 {
        let pose = motion.pose(at(rest, ms));
        assert!(pose.alpha <= last, "{ms} ms");
        assert!(
            pose.scale <= 1.0 && pose.scale >= EXIT_SCALE - 1e-6,
            "{pose:?}"
        );
        last = pose.alpha;
    }
    assert_eq!(motion.pose(at(rest, 200)).alpha, 0);
    assert!(!motion.exit_done(at(rest, 199)));
    assert!(motion.exit_done(at(rest, 200)));
    assert!(!motion.animating(at(rest, 200)));
}

#[test]
fn escape_during_the_entrance_fades_from_where_the_panel_is() {
    let base = Instant::now();
    let mut motion = ease();
    motion.start(base);
    let moment = at(base, 80);
    let on_screen = motion.pose(moment);
    assert!(on_screen.alpha > 0 && on_screen.alpha < 255);
    assert!(motion.begin_exit(moment));
    // The first frame of the exit is the last frame of the entrance.
    assert_eq!(motion.pose(moment), on_screen);
    // And it never brightens on the way down.
    let mut last = on_screen.alpha;
    for ms in 0..=200 {
        let alpha = motion.pose(at(moment, ms)).alpha;
        assert!(alpha <= last);
        last = alpha;
    }
    assert_eq!(last, 0);
}

#[test]
fn a_second_exit_keeps_the_first_start() {
    let base = Instant::now();
    let mut motion = ease();
    motion.start(base);
    assert!(motion.begin_exit(at(base, 300)));
    let mid = motion.pose(at(base, 400));
    assert!(motion.begin_exit(at(base, 350)));
    assert_eq!(motion.pose(at(base, 400)), mid);
}

#[test]
fn the_highlight_slides_to_a_moved_row_and_lands_on_it() {
    let base = Instant::now();
    let mut motion = ease();
    motion.set_highlight(Some(0.0), true, base);
    assert_eq!(motion.highlight_at(base), Some(0.0));
    assert!(!motion.animating(base), "the first row is not a move");
    motion.set_highlight(Some(46.0), true, base);
    assert_eq!(motion.highlight_at(base), Some(0.0), "starts where it was");
    assert!(motion.animating(base));
    let mid = motion.highlight_at(at(base, 100)).unwrap();
    assert!(mid > 0.0 && mid < 46.0, "{mid}");
    assert_eq!(motion.highlight_at(at(base, 200)), Some(46.0));
    assert!(!motion.animating(at(base, 200)));
}

#[test]
fn a_target_moved_mid_slide_continues_from_the_highlight_on_screen() {
    let base = Instant::now();
    let mut motion = ease();
    motion.set_highlight(Some(0.0), true, base);
    motion.set_highlight(Some(100.0), true, base);
    let moment = at(base, 60);
    let on_screen = motion.highlight_at(moment).unwrap();
    motion.set_highlight(Some(200.0), true, moment);
    assert_eq!(motion.highlight_at(moment), Some(on_screen));
    assert!(motion.highlight_at(at(moment, 100)).unwrap() > on_screen);
    assert_eq!(motion.highlight_at(at(moment, 200)), Some(200.0));
}

#[test]
fn the_same_target_again_leaves_a_slide_alone() {
    let base = Instant::now();
    let mut motion = ease();
    motion.set_highlight(Some(0.0), true, base);
    motion.set_highlight(Some(100.0), true, base);
    let mid = motion.highlight_at(at(base, 80));
    // A draw for any other reason names the same target.
    motion.set_highlight(Some(100.0), true, at(base, 80));
    assert_eq!(motion.highlight_at(at(base, 80)), mid);
}

#[test]
fn rows_that_changed_snap_and_a_vanished_selection_drops_the_highlight() {
    let base = Instant::now();
    let mut motion = ease();
    motion.set_highlight(Some(46.0), true, base);
    motion.set_highlight(Some(0.0), false, base);
    assert_eq!(motion.highlight_at(base), Some(0.0));
    assert!(!motion.animating(base));
    motion.set_highlight(None, true, base);
    assert_eq!(motion.highlight_at(base), None);
    // And a highlight that comes back from nothing does not slide.
    motion.set_highlight(Some(92.0), true, at(base, 10));
    assert_eq!(motion.highlight_at(at(base, 10)), Some(92.0));
    assert!(!motion.animating(at(base, 10)));
}

#[test]
fn a_non_finite_target_is_no_target() {
    let base = Instant::now();
    let mut motion = ease();
    motion.set_highlight(Some(f32::NAN), true, base);
    assert_eq!(motion.highlight_at(base), None);
    motion.set_highlight(Some(f32::INFINITY), true, base);
    assert_eq!(motion.highlight_at(base), None);
}

#[test]
fn a_late_frame_snaps_to_the_end_never_past_it() {
    let base = Instant::now();
    let mut motion = spring();
    motion.start(base);
    motion.set_highlight(Some(0.0), true, base);
    motion.set_highlight(Some(23.0), true, base);
    // The compositor stalled for ten seconds.
    let late = at(base, 10_000);
    assert_eq!(motion.pose(late), Pose::REST);
    assert_eq!(motion.highlight_at(late), Some(23.0));
    assert!(!motion.animating(late));
}

#[test]
fn the_slide_is_held_to_the_rows_and_inks_the_row_it_covers() {
    let base = Instant::now();
    let mut motion = spring();
    // Selected row 0 of 10 at 23 px rows, then row 9.
    let rest = slide(&mut motion, Some(0), 10, 23, true, base).unwrap();
    assert_eq!((rest.offset, rest.ink_slot), (0, 0));
    slide(&mut motion, Some(9), 10, 23, true, base);
    let mut previous_ink = 0;
    for ms in 0..=200 {
        let frame = slide(&mut motion, Some(9), 10, 23, true, at(base, ms)).unwrap();
        assert!(
            frame.offset <= 9 * 23,
            "{ms} ms: {frame:?} is past the last row"
        );
        assert!(frame.ink_slot < 10);
        assert!(
            frame.ink_slot + 1 >= previous_ink || ms > 150,
            "the ink follows the highlight down"
        );
        previous_ink = frame.ink_slot;
    }
    let landed = slide(&mut motion, Some(9), 10, 23, true, at(base, 200)).unwrap();
    assert_eq!((landed.offset, landed.ink_slot), (9 * 23, 9));
}

#[test]
fn the_ink_switches_when_the_highlight_is_half_way() {
    let base = Instant::now();
    let mut motion = ease();
    slide(&mut motion, Some(0), 10, 20, true, base);
    slide(&mut motion, Some(1), 10, 20, true, base);
    let early = slide(&mut motion, Some(1), 10, 20, true, at(base, 5)).unwrap();
    assert_eq!(early.ink_slot, 0);
    let late = slide(&mut motion, Some(1), 10, 20, true, at(base, 150)).unwrap();
    assert_eq!(late.ink_slot, 1);
}

#[test]
fn no_slide_without_motion_a_selection_or_rows() {
    let base = Instant::now();
    let mut off = Motion::new(Timing::NONE);
    assert_eq!(slide(&mut off, Some(0), 10, 23, true, base), None);
    let mut motion = ease();
    assert_eq!(slide(&mut motion, None, 10, 23, true, base), None);
    assert_eq!(slide(&mut motion, Some(0), 0, 23, true, base), None);
    // A selection scrolled out of the shown rows has no highlight.
    assert_eq!(slide(&mut motion, Some(12), 10, 23, true, base), None);
    // A zero row height cannot divide by zero.
    assert!(slide(&mut motion, Some(0), 10, 0, true, base).is_some());
}

#[test]
fn scrolling_the_selection_out_and_back_snaps_instead_of_sliding_from_nowhere() {
    let base = Instant::now();
    let mut motion = ease();
    slide(&mut motion, Some(3), 10, 20, true, base);
    assert_eq!(slide(&mut motion, None, 10, 20, true, at(base, 10)), None);
    let back = slide(&mut motion, Some(5), 10, 20, true, at(base, 20)).unwrap();
    assert_eq!(back.offset, 100);
    assert!(!motion.sliding(at(base, 20)));
}
