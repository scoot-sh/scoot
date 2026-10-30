use super::*;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn progress_clamps_and_a_zero_duration_snaps() {
    let t0 = Instant::now();
    assert_eq!(progress(t0, t0, Duration::ZERO), 1.0);
    assert_eq!(progress(t0, t0, ms(100)), 0.0);
    assert_eq!(progress(t0 + ms(50), t0, ms(100)), 0.5);
    assert_eq!(progress(t0 + ms(500), t0, ms(100)), 1.0);
    // A `now` before `start` (a clock sampled out of order) is not negative.
    assert_eq!(progress(t0, t0 + ms(10), ms(100)), 0.0);
}

#[test]
fn every_curve_starts_at_zero_and_lands_exactly_on_one() {
    for easing in [Easing::Linear, Easing::EaseOut, Easing::Spring] {
        assert_eq!(easing_factor(0.0, easing), 0.0, "{easing:?}");
        assert_eq!(easing_factor(1.0, easing), 1.0, "{easing:?}");
        // Out-of-range progress clamps instead of extrapolating.
        assert_eq!(easing_factor(-3.0, easing), 0.0, "{easing:?}");
        assert_eq!(easing_factor(9.0, easing), 1.0, "{easing:?}");
    }
}

#[test]
fn ease_out_is_the_cubic_and_never_overshoots() {
    assert!((easing_factor(0.5, Easing::EaseOut) - 0.875).abs() < 1e-6);
    let mut last = 0.0;
    for step in 0..=100 {
        let value = easing_factor(step as f32 / 100.0, Easing::EaseOut);
        assert!(value >= last && value <= 1.0);
        last = value;
    }
}

#[test]
fn the_spring_overshoots_about_nine_percent_then_settles() {
    let peak = (0..=1000)
        .map(|step| easing_factor(step as f32 / 1000.0, Easing::Spring))
        .fold(0.0f32, f32::max);
    assert!((1.08..1.10).contains(&peak), "{peak}");
}

#[test]
fn bounce_zero_is_ease_out_and_one_is_the_spring() {
    for step in 0..=64 {
        let p = step as f32 / 64.0;
        assert_eq!(bounce_factor(p, 0.0), easing_factor(p, Easing::EaseOut));
        let spring = easing_factor(p, Easing::Spring);
        assert!((bounce_factor(p, 1.0) - spring).abs() < 1e-6, "{p}");
    }
}

#[test]
fn bounce_scales_the_overshoot_and_always_lands() {
    let peak = |bounce: f32| {
        (0..=500)
            .map(|step| bounce_factor(step as f32 / 500.0, bounce))
            .fold(0.0f32, f32::max)
    };
    assert!(peak(0.0) <= 1.0);
    assert!(peak(0.25) < peak(0.5) && peak(0.5) < peak(1.0));
    for bounce in [0.0, 0.1, 0.5, 1.0] {
        assert_eq!(bounce_factor(0.0, bounce), 0.0);
        assert_eq!(bounce_factor(1.0, bounce), 1.0);
    }
}

#[test]
fn a_hostile_bounce_clamps_into_range() {
    for p in [0.0, 0.3, 0.7, 1.0] {
        assert_eq!(bounce_factor(p, 9.0), bounce_factor(p, 1.0));
        assert_eq!(bounce_factor(p, -9.0), bounce_factor(p, 0.0));
        assert_eq!(bounce_factor(p, f32::NAN), bounce_factor(p, 0.0));
        assert_eq!(bounce_factor(p, f32::INFINITY), bounce_factor(p, 1.0));
    }
}

#[test]
fn easing_names_round_trip_and_unknown_names_are_returned() {
    for easing in [Easing::Linear, Easing::EaseOut, Easing::Spring] {
        assert_eq!(Easing::parse(Some(easing.name())), Ok(easing));
    }
    assert_eq!(Easing::parse(None), Ok(Easing::EaseOut));
    assert_eq!(Easing::parse(Some("bouncy")), Err("bouncy".to_owned()));
}
