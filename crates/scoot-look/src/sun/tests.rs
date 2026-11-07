//! Sunrise and sunset against published values, the polar cases, the
//! flip search's edges, and the coordinate and calendar grammars.

use super::civil::{on_calendar_utc, unix_from_utc, utc_from_unix};
use super::{HORIZON_DEG, Location, altitude_deg, is_day, next_change, parse_degrees};

fn place(lat: f64, lon: f64) -> Location {
    Location::new(lat, lon).expect("in range")
}

/// One published day: where, the local civil date, the zone's offset from
/// UTC in hours (standard time, no daylight saving), and the almanac's
/// sunrise and sunset as local `HH:MM`.
struct Day {
    name: &'static str,
    lat: f64,
    lon: f64,
    date: (i64, i64, i64),
    offset: i64,
    rise: (i64, i64),
    set: (i64, i64),
}

/// Sunrise and sunset from the US Naval Observatory's Astronomical
/// Applications Department "Rise, Set, and Twilight Data" service
/// (<https://aa.usno.navy.mil/api/rstt/oneday?date=D&coords=LAT,LON&tz=H&dst=false>),
/// fetched 2026-11-07. The service reports the Sun's upper limb with
/// standard refraction, the convention [`HORIZON_DEG`] encodes.
/// Seconds of disagreement tolerated: the almanac prints whole minutes, so
/// the true time is within one minute of what it says. Measured against
/// these days the algorithm lands within 30 s of the printed minute.
const SLACK_SECS: i64 = 60;

const PUBLISHED: &[Day] = &[
    Day {
        name: "New York, summer solstice",
        lat: 40.7128,
        lon: -74.0060,
        date: (2024, 6, 21),
        offset: -4,
        rise: (5, 25),
        set: (20, 31),
    },
    Day {
        name: "London, winter solstice",
        lat: 51.5074,
        lon: -0.1278,
        date: (2024, 12, 21),
        offset: 0,
        rise: (8, 4),
        set: (15, 54),
    },
    Day {
        name: "Sydney, southern summer",
        lat: -33.8688,
        lon: 151.2093,
        date: (2024, 1, 15),
        offset: 10,
        rise: (4, 59),
        set: (19, 9),
    },
    Day {
        name: "Quito, equinox on the equator",
        lat: -0.1807,
        lon: -78.4678,
        date: (2024, 3, 20),
        offset: -5,
        rise: (6, 18),
        set: (18, 24),
    },
    Day {
        name: "Honolulu, equinox",
        lat: 21.3069,
        lon: -157.8583,
        date: (2024, 3, 20),
        offset: -10,
        rise: (6, 35),
        set: (18, 43),
    },
    Day {
        name: "Anchorage, winter solstice",
        lat: 61.2181,
        lon: -149.9003,
        date: (2024, 12, 21),
        offset: -9,
        rise: (10, 15),
        set: (15, 42),
    },
    Day {
        name: "Tromso, just after the polar night",
        lat: 69.6492,
        lon: 18.9553,
        date: (2024, 1, 21),
        offset: 1,
        rise: (10, 32),
        set: (13, 19),
    },
];

/// The two flips of one local civil day, as seconds after its local
/// midnight: the first sunrise and the first sunset after it.
fn events(day: &Day) -> (i64, i64) {
    let (y, m, d) = day.date;
    let midnight = unix_from_utc(y, m, d, 0, 0, 0) - day.offset * 3600;
    let p = place(day.lat, day.lon);
    assert!(!is_day(p, midnight), "{}: dark at local midnight", day.name);
    let rise = next_change(p, midnight, 86_400).expect("a sunrise");
    assert!(rise.day, "{}", day.name);
    let set = next_change(p, rise.at, 86_400).expect("a sunset");
    assert!(!set.day, "{}", day.name);
    (rise.at - midnight, set.at - midnight)
}

#[test]
fn sunrise_and_sunset_match_the_published_values() {
    for day in PUBLISHED {
        let (rise, set) = events(day);
        let want_rise = day.rise.0 * 3600 + day.rise.1 * 60;
        let want_set = day.set.0 * 3600 + day.set.1 * 60;
        // The almanac prints a minute: the true time lies within it.
        assert!(
            (rise - want_rise).abs() <= SLACK_SECS,
            "{}: sunrise {} s after local midnight, almanac {want_rise} s",
            day.name,
            rise
        );
        assert!(
            (set - want_set).abs() <= SLACK_SECS,
            "{}: sunset {} s after local midnight, almanac {want_set} s",
            day.name,
            set
        );
    }
}

#[test]
fn a_sunset_after_midnight_is_found_on_the_next_day() {
    // USNO, Reykjavik 2024-06-21 (64.1466 N, 21.9426 W, UTC+0): rise 02:55,
    // set 00:04 (the sun dips under the horizon for about three hours).
    let p = place(64.1466, -21.9426);
    let from = unix_from_utc(2024, 6, 21, 0, 30, 0);
    let rise = next_change(p, from, 86_400).expect("sunrise");
    assert!(rise.day);
    let want = unix_from_utc(2024, 6, 21, 2, 55, 0);
    assert!((rise.at - want).abs() <= SLACK_SECS, "{}", rise.at - want);
    let set = next_change(p, rise.at, 86_400).expect("sunset");
    assert!(!set.day);
    let want = unix_from_utc(2024, 6, 22, 0, 4, 0);
    assert!((set.at - want).abs() <= SLACK_SECS, "{}", set.at - want);
}

#[test]
fn polar_day_never_flips_and_is_always_day() {
    // USNO: Tromso 2024-06-21 "continuously above the horizon".
    let p = place(69.6492, 18.9553);
    let start = unix_from_utc(2024, 6, 20, 12, 0, 0);
    assert_eq!(next_change(p, start, 3 * 86_400), None);
    for minute in 0..3 * 24 * 60 {
        assert!(is_day(p, start + minute * 60), "minute {minute}");
    }
}

#[test]
fn polar_night_never_flips_and_is_always_night() {
    // USNO: Tromso 2024-12-21 "continuously below the horizon".
    let p = place(69.6492, 18.9553);
    let start = unix_from_utc(2024, 12, 20, 12, 0, 0);
    assert_eq!(next_change(p, start, 3 * 86_400), None);
    for minute in 0..3 * 24 * 60 {
        assert!(!is_day(p, start + minute * 60), "minute {minute}");
    }
    // The same at the other pole's midwinter.
    let south = place(-80.0, 0.0);
    let june = unix_from_utc(2024, 6, 21, 0, 0, 0);
    assert_eq!(next_change(south, june, 86_400), None);
    assert!(!is_day(south, june));
}

#[test]
fn the_polar_night_ends_with_one_sunrise_and_no_flicker_before_it() {
    let p = place(69.6492, 18.9553);
    let start = unix_from_utc(2023, 12, 21, 0, 0, 0);
    let first = next_change(p, start, 40 * 86_400).expect("the sun returns");
    assert!(
        first.day,
        "the first flip out of the polar night is a sunrise"
    );
    // USNO: Tromso 2024-01-21 has a sunrise; the first one is before it
    // and after the solstice.
    assert!(first.at > start && first.at < unix_from_utc(2024, 1, 21, 0, 0, 0));
    // Night until then, sampled every ten minutes.
    let mut t = start;
    while t < first.at {
        assert!(!is_day(p, t), "day at {t} before the first sunrise");
        t += 600;
    }
    assert!(is_day(p, first.at));
    assert!(!is_day(p, first.at - 1));
}

#[test]
fn the_flip_is_the_first_second_with_the_new_answer() {
    for day in PUBLISHED {
        let p = place(day.lat, day.lon);
        let (y, m, d) = day.date;
        let from = unix_from_utc(y, m, d, 0, 0, 0) - day.offset * 3600;
        let flip = next_change(p, from, 86_400).expect("a flip");
        assert_eq!(is_day(p, flip.at), flip.day, "{}", day.name);
        assert_eq!(is_day(p, flip.at - 1), !flip.day, "{}", day.name);
    }
}

#[test]
fn the_search_respects_its_span() {
    let p = place(51.5074, -0.1278);
    let midnight = unix_from_utc(2024, 12, 21, 0, 0, 0);
    // The sunrise is about eight hours out.
    assert_eq!(next_change(p, midnight, 0), None);
    assert_eq!(next_change(p, midnight, 7 * 3600), None);
    assert!(next_change(p, midnight, 9 * 3600).is_some());
    // A span that overflows is no span, not a panic.
    assert_eq!(next_change(p, i64::MAX - 5, 100), None);
    assert_eq!(next_change(p, midnight, i64::MAX), None);
    // The extremes of time are arithmetic, not a crash.
    let _ = is_day(p, i64::MIN);
    let _ = is_day(p, i64::MAX);
}

#[test]
fn the_search_is_the_same_from_any_starting_second() {
    // The flip does not depend on where the scan began.
    let p = place(48.8566, 2.3522);
    let a = next_change(p, unix_from_utc(2024, 3, 1, 0, 0, 0), 86_400).expect("flip");
    let b = next_change(p, a.at - 1, 86_400).expect("flip");
    let c = next_change(p, a.at - 3000, 86_400).expect("flip");
    assert_eq!(a, c);
    // From one second before, the very same flip is next.
    assert_eq!(a, b);
}

#[test]
fn solar_noon_at_the_equator_on_an_equinox_is_overhead() {
    let p = place(0.0, 0.0);
    let day = unix_from_utc(2024, 3, 20, 0, 0, 0);
    let peak = (0..24 * 60)
        .map(|m| altitude_deg(p, day + m * 60))
        .fold(f64::MIN, f64::max);
    // Within a degree: the equinox is at 03:06 UTC, noon is 3 h later.
    assert!(peak > 88.0 && peak <= 90.0, "{peak}");
    // And the horizon constant is the almanac's.
    assert!((HORIZON_DEG - -0.833).abs() < f64::EPSILON);
}

#[test]
fn the_poles_do_not_divide_by_zero() {
    for lat in [90.0, -90.0] {
        let p = place(lat, 0.0);
        for hour in 0..24 {
            let alt = altitude_deg(p, unix_from_utc(2024, 3, 20, hour, 0, 0));
            assert!(alt.is_finite() && alt.abs() <= 90.0, "{lat} {hour} {alt}");
        }
    }
}

// ---- coordinates ----

#[test]
fn degrees_take_one_plain_decimal_spelling() {
    for (text, want) in [
        ("0", 0.0),
        ("51.5", 51.5),
        ("-0.127758", -0.127758),
        ("90", 90.0),
        ("-180.000000", -180.0),
        ("007", 7.0),
        ("-0", 0.0),
    ] {
        assert_eq!(parse_degrees(text), Some(want), "`{text}`");
    }
    for bad in [
        "", "-", ".5", "5.", "+5", "1e1", "nan", "inf", "-inf", "0x10", " 5", "5 ", "5\n", "--5",
        "5,5", "٣", "5.5.5", "1_0", "-.5", "1/2",
    ] {
        assert_eq!(parse_degrees(bad), None, "`{bad}` must refuse");
    }
    // A huge digit string is refused, not parsed into infinity or slow.
    assert_eq!(parse_degrees(&"9".repeat(400)), None);
    assert_eq!(parse_degrees(&format!("0.{}", "1".repeat(400))), None);
}

#[test]
fn a_location_is_range_checked_and_prints_back_as_written() {
    assert!(Location::parse("90", "180").is_some());
    assert!(Location::parse("-90", "-180").is_some());
    assert!(Location::parse("90.000001", "0").is_none());
    assert!(Location::parse("0", "180.5").is_none());
    assert!(Location::parse("0", "-181").is_none());
    assert!(Location::parse("x", "0").is_none());
    assert!(Location::new(f64::NAN, 0.0).is_none());
    assert!(Location::new(0.0, f64::INFINITY).is_none());
    for (lat, lon, text) in [
        ("51.5074", "-0.1278", ("51.5074", "-0.1278")),
        ("51.50740000", "-0.12780", ("51.5074", "-0.1278")),
        ("-0", "0.0", ("0", "0")),
        ("-33", "151.2093", ("-33", "151.2093")),
        ("0.0000004", "0.0000006", ("0", "0.000001")),
        ("-0.0000004", "-180", ("0", "-180")),
        ("12.3456789", "1", ("12.345679", "1")),
    ] {
        let p = Location::parse(lat, lon).expect(lat);
        assert_eq!(
            (p.latitude_text().as_str(), p.longitude_text().as_str()),
            text
        );
        // Reading what was printed gives the same place.
        assert_eq!(
            Location::parse(&p.latitude_text(), &p.longitude_text()),
            Some(p)
        );
    }
}

// ---- calendar ----

#[test]
fn the_calendar_agrees_with_known_instants() {
    for (unix, want) in [
        (0, (1970, 1, 1, 0, 0, 0)),
        (86_399, (1970, 1, 1, 23, 59, 59)),
        (951_782_400, (2000, 2, 29, 0, 0, 0)),
        (1_709_164_800, (2024, 2, 29, 0, 0, 0)),
        (1_700_000_000, (2023, 11, 14, 22, 13, 20)),
        (4_107_542_400, (2100, 3, 1, 0, 0, 0)),
        (-1, (1969, 12, 31, 23, 59, 59)),
        (-86_400 * 365, (1969, 1, 1, 0, 0, 0)),
    ] {
        assert_eq!(utc_from_unix(unix), want, "{unix}");
        let (y, mo, d, h, mi, s) = want;
        assert_eq!(unix_from_utc(y, mo, d, h, mi, s), unix);
    }
}

#[test]
fn the_calendar_round_trips_across_centuries() {
    let mut t = unix_from_utc(1900, 1, 1, 0, 0, 0);
    let end = unix_from_utc(2200, 1, 1, 0, 0, 0);
    while t < end {
        let (y, mo, d, h, mi, s) = utc_from_unix(t);
        assert_eq!(unix_from_utc(y, mo, d, h, mi, s), t);
        t += 86_400 * 7 + 3_661;
    }
}

#[test]
fn on_calendar_names_the_instant_in_utc() {
    assert_eq!(
        on_calendar_utc(1_700_000_000).as_deref(),
        Some("2023-11-14 22:13:20 UTC")
    );
    assert_eq!(
        on_calendar_utc(0).as_deref(),
        Some("1970-01-01 00:00:00 UTC")
    );
    assert_eq!(on_calendar_utc(-86_400 * 365 * 3000), None);
    assert_eq!(on_calendar_utc(i64::MAX), None);
    assert_eq!(on_calendar_utc(i64::MIN), None);
}
