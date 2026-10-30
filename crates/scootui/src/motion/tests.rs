use super::*;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn a_look_without_motion_snaps_everything() {
    let t0 = Instant::now();
    let none = Timing::NONE;
    assert!(!none.active());
    assert!(none.settled(t0, t0));
    assert_eq!(none.eased(t0, t0), 1.0);
    assert_eq!(Settle::new(40.0, t0).value(&none, t0), 0.0);
    assert!(Settle::new(40.0, t0).settled(&none, t0));
}

#[test]
fn zero_duration_and_curve_none_both_collapse_to_no_motion() {
    assert_eq!(Timing::new(Curve::Spring, 0, 1.0), Timing::NONE);
    assert_eq!(Timing::new(Curve::EaseOut, 0, 0.5), Timing::NONE);
    assert_eq!(Timing::new(Curve::None, 300, 1.0), Timing::NONE);
}

#[test]
fn inputs_clamp() {
    let long = Timing::new(Curve::EaseOut, u32::MAX, 5.0);
    assert_eq!(long.duration(), ms(u64::from(MAX_DURATION_MS)));
    let spring = Timing::new(Curve::Spring, 100, 9.0);
    assert_eq!(spring.bounce, 1.0);
    assert_eq!(Timing::new(Curve::Spring, 100, -1.0).bounce, 0.0);
    assert_eq!(Timing::new(Curve::Spring, 100, f32::NAN).bounce, 0.0);
}

#[test]
fn a_settle_runs_from_the_gap_to_exactly_zero() {
    let t0 = Instant::now();
    let timing = Timing::new(Curve::EaseOut, 200, 0.0);
    let settle = Settle::new(100.0, t0);
    assert_eq!(settle.value(&timing, t0), 100.0);
    // Ease-out at the halfway point has covered 7/8 of the distance.
    assert!((settle.value(&timing, t0 + ms(100)) - 12.5).abs() < 1e-3);
    assert_eq!(settle.value(&timing, t0 + ms(200)), 0.0);
    assert!(settle.settled(&timing, t0 + ms(200)));
    assert!(!settle.settled(&timing, t0 + ms(199)));
    // A frame long after the deadline still lands exactly.
    assert_eq!(settle.value(&timing, t0 + Duration::from_secs(60)), 0.0);
}

#[test]
fn a_spring_overshoots_past_zero_and_a_zero_bounce_does_not() {
    let t0 = Instant::now();
    let spring = Timing::new(Curve::Spring, 400, 1.0);
    let settle = Settle::new(100.0, t0);
    let lowest = (0..=400)
        .map(|at| settle.value(&spring, t0 + ms(at)))
        .fold(f32::MAX, f32::min);
    assert!(lowest < -8.0 && lowest > -10.0, "{lowest}");
    let flat = Timing::new(Curve::Spring, 400, 0.0);
    let lowest = (0..=400)
        .map(|at| settle.value(&flat, t0 + ms(at)))
        .fold(f32::MAX, f32::min);
    assert!(lowest >= 0.0, "{lowest}");
}

#[test]
fn a_nonfinite_gap_is_no_gap() {
    let t0 = Instant::now();
    let timing = Timing::new(Curve::EaseOut, 200, 0.0);
    for gap in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(Settle::new(gap, t0).value(&timing, t0), 0.0);
    }
}

#[test]
fn leaving_never_overshoots() {
    let leaving = Timing::new(Curve::Spring, 400, 1.0).without_overshoot();
    assert_eq!(leaving.curve(), Curve::EaseOut);
    assert_eq!(leaving.duration(), ms(400));
    assert_eq!(Timing::NONE.without_overshoot(), Timing::NONE);
}

#[test]
fn curve_names_parse_and_unknown_ones_do_not() {
    assert_eq!(Curve::parse("none"), Some(Curve::None));
    assert_eq!(Curve::parse("ease-out"), Some(Curve::EaseOut));
    assert_eq!(Curve::parse("spring"), Some(Curve::Spring));
    for bad in ["", "Spring", "linear", "ease_out"] {
        assert_eq!(Curve::parse(bad), None, "{bad}");
    }
}

#[test]
fn a_settle_allocates_nothing() {
    let t0 = Instant::now();
    let timing = Timing::new(Curve::Spring, 300, 0.6);
    let settle = Settle::new(64.0, t0);
    let (_, allocs) = scootbg_mem::count_allocations(|| {
        let mut sum = 0.0;
        for at in 0..400 {
            sum += settle.value(&timing, t0 + ms(at));
        }
        sum
    });
    assert_eq!(allocs, 0);
}
