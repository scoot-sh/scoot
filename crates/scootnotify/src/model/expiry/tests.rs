//! The clock's contract: defaults, clamps, pause and resume, all over
//! an explicit `now` (no sleeping in tests).

use std::time::{Duration, Instant};

use crate::bus::hints::Urgency;
use crate::config::Config;

use super::{Clock, resolve_timeout};

fn now() -> Instant {
    Instant::now()
}

fn config() -> Config {
    Config::default()
}

#[test]
fn negative_is_the_urgency_default() {
    assert_eq!(resolve_timeout(-1, Urgency::Low, &config()), Some(5_000));
    assert_eq!(
        resolve_timeout(-1, Urgency::Normal, &config()),
        Some(10_000)
    );
    assert_eq!(resolve_timeout(-1, Urgency::Critical, &config()), None);
    // Any other negative reads as -1, not a second protocol.
    assert_eq!(
        resolve_timeout(-7, Urgency::Normal, &config()),
        Some(10_000)
    );
}

#[test]
fn zero_is_never() {
    assert_eq!(resolve_timeout(0, Urgency::Low, &config()), None);
    assert_eq!(resolve_timeout(0, Urgency::Critical, &config()), None);
}

#[test]
fn a_positive_value_is_honored_to_the_clamp() {
    assert_eq!(
        resolve_timeout(3000, Urgency::Critical, &config()),
        Some(3000)
    );
    assert_eq!(
        resolve_timeout(i32::MAX, Urgency::Normal, &config()),
        Some(3_600_000)
    );
}

#[test]
fn the_clock_starts_at_presentation_not_arrival() {
    let start = now();
    let mut clock = Clock::waiting();
    assert!(!clock.shown());
    assert!(!clock.due(start + Duration::from_secs(3600)));
    assert_eq!(clock.deadline(), None);
    clock.presented(start, Some(100));
    assert!(clock.shown());
    assert!(!clock.due(start + Duration::from_millis(99)));
    assert!(clock.due(start + Duration::from_millis(100)));
    // Re-presenting is a no-op; replace restarts instead.
    clock.presented(start + Duration::from_millis(50), Some(10_000));
    assert!(clock.due(start + Duration::from_millis(100)));
    clock.restart(start + Duration::from_millis(50), Some(10_000));
    assert!(!clock.due(start + Duration::from_millis(100)));
}

#[test]
fn never_has_no_deadline() {
    let start = now();
    let mut clock = Clock::waiting();
    clock.presented(start, None);
    assert!(clock.shown());
    assert_eq!(clock.deadline(), None);
    assert!(!clock.due(start + Duration::from_secs(86_400)));
}

#[test]
fn pause_parks_and_resume_continues() {
    let start = now();
    let mut clock = Clock::waiting();
    clock.presented(start, Some(1000));
    clock.pause(start + Duration::from_millis(400));
    // Parked: no deadline, nothing due, however long idle lasts.
    assert_eq!(clock.deadline(), None);
    assert!(!clock.due(start + Duration::from_secs(3600)));
    clock.resume(start + Duration::from_millis(10_000));
    assert!(!clock.due(start + Duration::from_millis(10_000) + Duration::from_millis(599)));
    assert!(clock.due(start + Duration::from_millis(10_000) + Duration::from_millis(600)));
}

/// Pausing twice parks once: the second pause must not re-park from a
/// `None` deadline to a zero remaining (which would expire at once on
/// resume). Reverting `pause`'s early return fails this.
#[test]
fn pause_is_idempotent() {
    let start = now();
    let mut clock = Clock::waiting();
    clock.presented(start, Some(1000));
    clock.pause(start + Duration::from_millis(400));
    clock.pause(start + Duration::from_millis(500));
    clock.resume(start + Duration::from_millis(10_000));
    assert!(!clock.due(start + Duration::from_millis(10_000) + Duration::from_millis(599)));
    assert!(clock.due(start + Duration::from_millis(10_000) + Duration::from_millis(600)));
}

#[test]
fn resume_without_pause_changes_nothing() {
    let start = now();
    let mut clock = Clock::waiting();
    clock.presented(start, Some(100));
    clock.resume(start + Duration::from_millis(50));
    assert!(clock.due(start + Duration::from_millis(100)));
}
