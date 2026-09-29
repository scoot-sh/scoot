use std::time::{Duration, Instant};

use rustix::event::PollFlags;

use super::format::Format;
use super::tzif::{Tz, days_from_civil};
use super::zone::Spec;
use super::{Clock, Period, Settings, Span};
use crate::modules::harness::Harness;
use crate::modules::{Module, OutputView, Update, View};

const NEW_YORK: &[u8] = include_bytes!("fixtures/America_New_York.slim.tzif");
const LORD_HOWE: &[u8] = include_bytes!("fixtures/Australia_Lord_Howe.fat.tzif");
const SYDNEY: &[u8] = include_bytes!("fixtures/Australia_Sydney.slim.tzif");

fn utc(year: i64, month: u32, day: u32, hour: i64, minute: i64, second: i64) -> i64 {
    days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second
}

fn clock(format: &str, tz: Tz) -> Clock {
    let settings = Settings {
        format: Format::parse(format).unwrap(),
    };
    let mut clock = Clock::new(settings, Spec::Utc).unwrap();
    clock.tz = tz;
    clock
}

/// What `clock` shows at `t`, and when it next changes.
fn at(clock: &mut Clock, t: i64) -> (String, Span) {
    let span = clock.render(t);
    (clock.scratch.clone(), span)
}

#[test]
fn a_span_is_the_local_minute_or_second() {
    let t = utc(2026, 9, 29, 15, 7, 42);
    assert_eq!(
        Span::containing(t, 0, Period::Minute),
        Span {
            start: t - 42,
            next: t + 18
        }
    );
    assert_eq!(
        Span::containing(t, 0, Period::Second),
        Span {
            start: t,
            next: t + 1
        }
    );
    // An offset with seconds (old local mean times) moves the boundary.
    let lmt = -17_762; // New York's LMT, -4:56:02
    let span = Span::containing(t, lmt, Period::Minute);
    assert_eq!((span.start + i64::from(lmt)).rem_euclid(60), 0);
    assert!(span.contains(t) && !span.contains(span.next));
    // The extremes clamp rather than overflow.
    let far = Span::containing(i64::MAX, 93_599, Period::Minute);
    assert!(far.start < far.next);
}

/// Summer time ends: the first tick after 05:59:59 UTC shows 1:00 am
/// again, now EST, and the span it arms for ends a minute later.
#[test]
fn the_clock_falls_back_on_the_boundary() {
    let mut clock = clock("%-I:%M %P %Z", Tz::parse(NEW_YORK).unwrap());
    let before = utc(2026, 11, 1, 5, 59, 30);
    let (text, span) = at(&mut clock, before);
    assert_eq!(text, "1:59 am EDT");
    assert_eq!(span.next, utc(2026, 11, 1, 6, 0, 0));
    let (text, span) = at(&mut clock, span.next);
    assert_eq!(text, "1:00 am EST");
    assert_eq!(span.next, utc(2026, 11, 1, 6, 1, 0));
}

#[test]
fn the_clock_springs_forward_on_the_boundary() {
    let mut clock = clock("%H:%M %Z", Tz::parse(NEW_YORK).unwrap());
    let (text, span) = at(&mut clock, utc(2026, 3, 8, 6, 59, 59));
    assert_eq!(text, "01:59 EST");
    let (text, _) = at(&mut clock, span.next);
    assert_eq!(text, "03:00 EDT");
}

/// Lord Howe moves by half an hour; Sydney's summer spans the new year.
#[test]
fn half_hour_and_southern_summers() {
    let mut howe = clock("%H:%M %z", Tz::parse(LORD_HOWE).unwrap());
    // 2026-10-04 02:00 local (+1030) is 2026-10-03 15:30 UTC.
    let (text, span) = at(&mut howe, utc(2026, 10, 3, 15, 29, 59));
    assert_eq!(text, "01:59 +1030");
    let (text, _) = at(&mut howe, span.next);
    assert_eq!(text, "02:30 +1100");
    let mut sydney = clock("%d %b %H:%M %Z", Tz::parse(SYDNEY).unwrap());
    let (text, _) = at(&mut sydney, utc(2026, 12, 31, 13, 0, 0));
    assert_eq!(text, "01 Jan 00:00 AEDT");
    let (text, _) = at(&mut sydney, utc(2027, 6, 30, 14, 0, 0));
    assert_eq!(text, "01 Jul 00:00 AEST");
}

/// A clock step between two ticks: the next refresh shows the new time,
/// wherever it lands, and arms for the boundary after it.
#[test]
fn a_step_shows_at_once_and_rearms_for_the_new_time() {
    let mut clock = clock("%H:%M", Tz::utc());
    let (text, span) = at(&mut clock, utc(2026, 9, 29, 15, 7, 10));
    assert_eq!(text, "15:07");
    // Stepped back an hour mid-minute.
    let stepped = utc(2026, 9, 29, 14, 7, 40);
    assert!(!span.contains(stepped));
    let (text, span) = at(&mut clock, stepped);
    assert_eq!(text, "14:07");
    assert_eq!(span.next, utc(2026, 9, 29, 14, 8, 0));
}

#[test]
fn the_default_setting_is_the_twelve_hour_clock() {
    assert_eq!(Settings::default().format, Format::default());
}

/// The real timer: a clock showing seconds is readable within about a
/// second, reports a change, and is armed for the next second after it.
#[test]
fn the_timer_fires_on_the_next_boundary() {
    let clock = clock("%H:%M:%S", Tz::utc());
    let mut harness = Harness::new(Box::new(clock));
    let first = harness.view().text().to_owned();
    let start = Instant::now();
    let mut update = Update::Unchanged;
    while update == Update::Unchanged && start.elapsed() < Duration::from_secs(3) {
        update = harness
            .wait(Duration::from_millis(1500))
            .unwrap_or(Update::Unchanged);
    }
    assert_eq!(update, Update::Changed, "no tick within 3 s");
    assert!(start.elapsed() < Duration::from_millis(2100));
    assert_ne!(harness.view().text(), first);
}

/// A minute clock arms its timer for the next minute boundary: the
/// time left is at most a minute, and ends on a whole minute.
#[test]
fn a_minute_clock_is_armed_for_the_next_minute() {
    let clock = clock("%H:%M", Tz::utc());
    let spec = rustix::time::timerfd_gettime(&clock.timer).unwrap();
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Realtime);
    let left = spec.it_value.tv_sec as f64 + spec.it_value.tv_nsec as f64 / 1e9;
    assert!(left > 0.0 && left <= 60.0, "{left}");
    let deadline = now.tv_sec as f64 + now.tv_nsec as f64 / 1e9 + left;
    let off = deadline - (deadline / 60.0).round() * 60.0;
    assert!(
        off.abs() < 0.05,
        "deadline {deadline} is {off} s off a minute"
    );
    assert_eq!(spec.it_interval.tv_sec, 0);
}

/// A spurious wake (nothing to read) changes nothing and re-arms.
#[test]
fn a_spurious_wake_is_harmless() {
    let mut clock = clock("%H:%M", Tz::utc());
    let before = clock.text.clone();
    let update = clock.on_ready(0, PollFlags::IN);
    // Within the same minute nothing changed (a minute boundary may pass
    // during the test, which is a change).
    if clock.text == before {
        assert_eq!(update, Update::Unchanged);
    }
    let mut view = View::default();
    clock.view(&OutputView { name: None }, &mut view);
    assert_eq!(view.text(), clock.text);
}

/// The zone file is read again when it changes on disk.
#[test]
fn a_changed_zone_file_is_read_again() {
    let dir = std::env::temp_dir().join(format!("scootbar-clock-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("localtime");
    std::fs::write(&path, NEW_YORK).unwrap();
    let settings = Settings {
        format: Format::parse("%Z").unwrap(),
    };
    let spec = Spec::File {
        path: path.clone(),
        fallback: None,
    };
    let mut clock = Clock::new(settings, spec).unwrap();
    assert!(clock.text == "EDT" || clock.text == "EST", "{}", clock.text);
    // Replaced by another zone, atomically.
    let next = dir.join("next");
    std::fs::write(&next, include_bytes!("fixtures/Asia_Kolkata.fat.tzif")).unwrap();
    std::fs::rename(&next, &path).unwrap();
    let update = clock.on_ready(0, PollFlags::IN);
    assert_eq!(update, Update::Changed);
    assert_eq!(clock.text, "IST");
    // Gone: the zone last read stays.
    std::fs::remove_file(&path).unwrap();
    let _ = clock.on_ready(0, PollFlags::IN);
    assert_eq!(clock.text, "IST");
    let _ = std::fs::remove_dir_all(&dir);
}
