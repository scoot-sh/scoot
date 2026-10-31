//! The accumulator and frame helpers, with synthetic clocks and no seat.
//! The wire-level behavior (what a client receives, what a bind fires) is
//! pinned in `input/tests/scroll_binds.rs`.

use smithay::backend::input::{AxisSource, InputTime};
use smithay::input::pointer::AxisFrame;

use super::*;

fn clock() -> Instant {
    Instant::now()
}

#[test]
fn whole_steps_fire_and_the_remainder_carries() {
    let mut axis = Accumulator::default();
    let t = clock();
    assert_eq!(axis.feed(25.0, Unit::Motion, 60.0, 8, t), 0);
    assert_eq!(axis.feed(25.0, Unit::Motion, 60.0, 8, t), 0);
    // 75 so far: one step, 15 left.
    assert_eq!(axis.feed(25.0, Unit::Motion, 60.0, 8, t), 1);
    assert_eq!(axis.feed(45.0, Unit::Motion, 60.0, 8, t), 1, "15 + 45 = 60");
    assert_eq!(
        axis.feed(130.0, Unit::Motion, 60.0, 8, t),
        2,
        "130 = 2 * 60 + 10"
    );
    assert!((axis.amount - 10.0).abs() < 1e-9, "left: {}", axis.amount);
}

#[test]
fn a_negative_run_counts_the_same() {
    let mut axis = Accumulator::default();
    let t = clock();
    assert_eq!(axis.feed(-70.0, Unit::Motion, 60.0, 8, t), 1);
    assert!((axis.amount + 10.0).abs() < 1e-9);
    assert_eq!(axis.feed(-50.0, Unit::Motion, 60.0, 8, t), 1);
}

#[test]
fn a_reversal_discards_the_partial_step() {
    let mut axis = Accumulator::default();
    let t = clock();
    assert_eq!(axis.feed(50.0, Unit::Motion, 60.0, 8, t), 0);
    // Without the reset, -50 would cancel to 0 and the next -20 would not
    // fire; with it, -50 starts over and -20 more makes 70.
    assert_eq!(axis.feed(-50.0, Unit::Motion, 60.0, 8, t), 0);
    assert_eq!(axis.feed(-20.0, Unit::Motion, 60.0, 8, t), 1);
}

#[test]
fn an_idle_axis_forgets_its_partial_step() {
    let mut axis = Accumulator::default();
    let t = clock();
    assert_eq!(axis.feed(50.0, Unit::Motion, 60.0, 8, t), 0);
    // Just inside the window: the 50 is still there.
    let soon = t + IDLE_RESET;
    assert_eq!(axis.feed(20.0, Unit::Motion, 60.0, 8, soon), 1);
    let mut axis = Accumulator::default();
    assert_eq!(axis.feed(50.0, Unit::Motion, 60.0, 8, t), 0);
    let late = t + IDLE_RESET + Duration::from_millis(1);
    assert_eq!(axis.feed(20.0, Unit::Motion, 60.0, 8, late), 0);
}

#[test]
fn a_clock_that_runs_backwards_does_not_panic_or_reset() {
    let mut axis = Accumulator::default();
    let t = clock() + Duration::from_secs(5);
    axis.feed(50.0, Unit::Motion, 60.0, 8, t);
    assert_eq!(
        axis.feed(20.0, Unit::Motion, 60.0, 8, t - Duration::from_secs(1)),
        1
    );
}

#[test]
fn a_hi_res_wheel_accumulates_to_one_detent() {
    let mut axis = Accumulator::default();
    let t = clock();
    for _ in 0..3 {
        assert_eq!(axis.feed(30.0, Unit::Detents, DETENT, 8, t), 0);
    }
    assert_eq!(axis.feed(30.0, Unit::Detents, DETENT, 8, t), 1);
    assert_eq!(
        axis.feed(120.0, Unit::Detents, DETENT, 8, t),
        1,
        "a classic click"
    );
}

#[test]
fn changing_unit_starts_over() {
    let mut axis = Accumulator::default();
    let t = clock();
    axis.feed(100.0, Unit::Detents, DETENT, 8, t);
    // 100 detent-units must not leak into a motion count.
    assert_eq!(axis.feed(30.0, Unit::Motion, 60.0, 8, t), 0);
}

#[test]
fn one_event_fires_at_most_the_cap() {
    let mut axis = Accumulator::default();
    let t = clock();
    assert_eq!(axis.feed(60.0 * 100.0, Unit::Motion, 60.0, 8, t), 8);
    assert!(
        axis.amount.abs() < 60.0,
        "the dropped steps are not deferred"
    );
    assert_eq!(axis.feed(1e300, Unit::Motion, 60.0, 8, t), 8);
    assert_eq!(axis.feed(f64::MAX, Unit::Detents, DETENT, 1, t), 1);
}

#[test]
fn a_huge_delta_leaves_less_than_one_step_behind() {
    // The step count is capped, but the remainder is a separate promise: it
    // must be under one step with the delta's sign, not the rounding error of
    // `whole * threshold` at 1e300 (which left a residual of the same order
    // and made every later event fire at the cap).
    for delta in [
        1e300,
        -1e300,
        f64::MAX,
        f64::MIN,
        6.0e5 + 17.0,
        -6.0e5 - 17.0,
    ] {
        let mut axis = Accumulator::default();
        let t = clock();
        assert_eq!(axis.feed(delta, Unit::Motion, 60.0, 8, t), 8, "{delta}");
        assert!(
            axis.amount.abs() < 60.0,
            "{delta} left a remainder of {}",
            axis.amount
        );
        assert!(
            axis.amount == 0.0 || axis.amount.signum() == delta.signum(),
            "{delta} flipped the remainder's sign: {}",
            axis.amount
        );
        // And the next small event behaves as on a fresh axis.
        let fresh = Accumulator::default().feed(10.0, Unit::Motion, 60.0, 8, t);
        let after = axis.feed(delta.signum() * 10.0, Unit::Motion, 60.0, 8, t);
        assert!(after <= 1 && fresh == 0, "{delta}: {after} steps after");
    }
    // An exact multiple leaves exactly nothing.
    let mut axis = Accumulator::default();
    assert_eq!(axis.feed(120.0, Unit::Motion, 60.0, 8, clock()), 2);
    assert_eq!(axis.amount, 0.0);
}

#[test]
fn hostile_values_fire_nothing() {
    let mut axis = Accumulator::default();
    let t = clock();
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0] {
        assert_eq!(axis.feed(bad, Unit::Motion, 60.0, 8, t), 0);
    }
    assert_eq!(
        axis.feed(60.0, Unit::Motion, 0.0, 8, t),
        0,
        "a zero threshold"
    );
    assert_eq!(axis.feed(60.0, Unit::Motion, f64::NAN, 8, t), 0);
    assert_eq!(axis.feed(60.0, Unit::Motion, -5.0, 8, t), 0);
    assert_eq!(axis.amount, 0.0);
}

#[test]
fn reset_keeps_a_claim() {
    let mut axis = Accumulator {
        claimed_at: Some(clock()),
        ..Accumulator::default()
    };
    axis.feed(30.0, Unit::Motion, 60.0, 8, clock());
    axis.reset();
    assert_eq!(axis.amount, 0.0);
    assert!(axis.claimed_at.is_some());
}

fn frame(h: f64, v: f64, h120: i32, v120: i32, stop: (bool, bool)) -> AxisFrame {
    let mut frame = AxisFrame::new(InputTime::from_millis(1)).source(AxisSource::Wheel);
    frame.axis = (h, v);
    if h120 != 0 || v120 != 0 {
        frame.v120 = Some((h120, v120));
    }
    frame.stop = stop;
    frame
}

#[test]
fn direction_follows_the_sign_wheel_steps_first() {
    let read = |f: &AxisFrame, vertical| AxisReading::of(f, vertical).direction(vertical);
    let f = frame(-3.0, 4.0, 0, 0, (false, false));
    assert_eq!(read(&f, false), Some(ScrollDir::Left));
    assert_eq!(read(&f, true), Some(ScrollDir::Down));
    let f = frame(3.0, -4.0, 0, 0, (false, false));
    assert_eq!(read(&f, false), Some(ScrollDir::Right));
    assert_eq!(read(&f, true), Some(ScrollDir::Up));
    // The step count wins over a value of the opposite sign.
    let f = frame(0.0, 5.0, 0, -120, (false, false));
    assert_eq!(read(&f, true), Some(ScrollDir::Up));
    // Nothing moved, or nothing finite.
    let f = frame(0.0, f64::NAN, 0, 0, (false, true));
    assert_eq!(read(&f, true), None);
    assert_eq!(read(&f, false), None);
}

#[test]
fn strip_removes_one_axis_and_leaves_the_other() {
    let mut f = frame(2.0, 3.0, 120, 240, (true, true));
    strip(&mut f, true);
    assert_eq!(f.axis, (2.0, 0.0));
    assert_eq!(f.v120, Some((120, 0)));
    assert_eq!(f.stop, (true, false));
    assert!(has_content(&f));
    strip(&mut f, false);
    assert!(!has_content(&f), "{f:?}");
}

#[test]
fn an_empty_frame_has_no_content() {
    assert!(!has_content(&frame(0.0, 0.0, 0, 0, (false, false))));
    assert!(has_content(&frame(0.0, 0.0, 0, 0, (false, true))));
    assert!(has_content(&frame(0.1, 0.0, 0, 0, (false, false))));
}

#[test]
fn locked_sessions_fire_only_flagged_spawns() {
    let plain = BindFlags::default();
    let allowed = BindFlags {
        allow_when_locked: true,
        ..BindFlags::default()
    };
    let spawn = Action::Spawn(vec!["true".into()]);
    assert!(bind_fires(false, &Action::CloseFocused, plain));
    assert!(bind_fires(false, &spawn, plain));
    assert!(!bind_fires(true, &spawn, plain), "a spawn needs the flag");
    assert!(bind_fires(true, &spawn, allowed));
    assert!(
        !bind_fires(true, &Action::CloseFocused, allowed),
        "the flag only counts for a spawn"
    );
}

#[test]
fn a_claim_lapses_only_after_the_expiry() {
    let t = clock();
    let mut axis = Accumulator {
        claimed_at: Some(t),
        ..Accumulator::default()
    };
    axis.expire_claim(t + CLAIM_EXPIRY);
    assert!(axis.claimed_at.is_some(), "still within the window");
    axis.expire_claim(t + CLAIM_EXPIRY + Duration::from_millis(1));
    assert!(
        axis.claimed_at.is_none(),
        "a lost stop must not latch forever"
    );
    axis.expire_claim(t);
    assert!(axis.claimed_at.is_none());
}
