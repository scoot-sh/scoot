//! Tests for the popup entrance: the slide, the room, and the settle.

use std::time::{Duration, Instant};

use scootui::motion::{Curve, Timing};

use crate::bar::Edge;
use crate::popup_motion::{MOTION_ROOM, SLIDE_PX, entrance_dy, motion_pad, settled};

fn spring() -> Timing {
    Timing::new(Curve::Spring, 250, 1.0)
}

fn ease() -> Timing {
    Timing::new(Curve::EaseOut, 200, 0.0)
}

#[test]
fn none_moves_nothing_and_needs_no_room() {
    let timing = Timing::NONE;
    let start = Instant::now();
    assert_eq!(motion_pad(&timing), (0, 0, 0, 0));
    for edge in [Edge::Top, Edge::Bottom] {
        assert_eq!(entrance_dy(&timing, start, start, edge), 0.0);
        assert_eq!(
            entrance_dy(&timing, start, start + Duration::from_millis(1000), edge),
            0.0
        );
        assert!(settled(&timing, start, start,));
        assert!(settled(&timing, start, start + Duration::from_millis(1000)));
    }
}

#[test]
fn active_needs_room_on_every_side() {
    for timing in [spring(), ease()] {
        assert!(timing.active());
        assert_eq!(
            motion_pad(&timing),
            (MOTION_ROOM, MOTION_ROOM, MOTION_ROOM, MOTION_ROOM)
        );
    }
    // The slide fits its room: a spring's overshoot stays inside it.
    const { assert!(MOTION_ROOM >= SLIDE_PX) };
}

#[test]
fn entrance_starts_a_slide_away_and_lands_on_zero() {
    for edge in [Edge::Top, Edge::Bottom] {
        let timing = ease();
        let start = Instant::now();
        let at_start = entrance_dy(&timing, start, start, edge);
        assert_eq!(
            at_start.abs(),
            SLIDE_PX as f32,
            "{edge:?} starts a slide away"
        );
        match edge {
            Edge::Top => assert!(at_start < 0.0, "a top popup starts above rest"),
            Edge::Bottom => assert!(at_start > 0.0, "a bottom popup starts below rest"),
        }
        assert!(!settled(&timing, start, start));
        let end = start + Duration::from_millis(200);
        assert_eq!(entrance_dy(&timing, start, end, edge), 0.0);
        assert!(settled(&timing, start, end));
        let past = start + Duration::from_millis(1000);
        assert_eq!(entrance_dy(&timing, start, past, edge), 0.0);
        assert!(settled(&timing, start, past));
    }
}

#[test]
fn entrance_moves_toward_rest_mid_flight() {
    let timing = ease();
    let start = Instant::now();
    let mid = start + Duration::from_millis(100);
    for edge in [Edge::Top, Edge::Bottom] {
        let at_start = entrance_dy(&timing, start, start, edge).abs();
        let at_mid = entrance_dy(&timing, start, mid, edge).abs();
        assert!(
            at_mid < at_start,
            "{edge:?} moves toward rest: {at_mid} !< {at_start}"
        );
        assert!(!settled(&timing, start, mid));
    }
}

#[test]
fn a_zero_duration_is_no_motion() {
    let timing = Timing::new(Curve::Spring, 0, 1.0);
    assert_eq!(timing, Timing::NONE);
    assert_eq!(motion_pad(&timing), (0, 0, 0, 0));
    let start = Instant::now();
    assert_eq!(entrance_dy(&timing, start, start, Edge::Top), 0.0);
    assert!(settled(&timing, start, start));
}

/// The motion room is part of the surface cache key: `none` keeps the
/// surface it always had, active reserves room, so a runtime look switch
/// that flips active/inactive remakes the popup (the total pad changes).
/// A switch that changes the curve with the same room restarts the
/// entrance instead (see `restyle_open`).
#[test]
fn motion_room_keys_the_surface_and_timing_keys_the_restart() {
    assert_eq!(motion_pad(&Timing::NONE), (0, 0, 0, 0));
    assert_eq!(
        motion_pad(&spring()),
        (MOTION_ROOM, MOTION_ROOM, MOTION_ROOM, MOTION_ROOM)
    );
    assert_eq!(
        motion_pad(&ease()),
        (MOTION_ROOM, MOTION_ROOM, MOTION_ROOM, MOTION_ROOM)
    );
    // Different curves are different timings: a look switch rebuilds.
    assert_ne!(spring(), ease());
    assert_ne!(Timing::NONE, spring());
}
