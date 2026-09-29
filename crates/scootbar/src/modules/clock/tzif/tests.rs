//! The reader against `zdump` (fixtures, `../fixtures/generate.sh`), hand-
//! written footers for the forms tzdata never uses, and malformed input.

use super::{Abbr, Local, MAX_FILE, MAX_INSTANT, Tz, civil_from_days, days_from_civil, weekday};

macro_rules! zone {
    ($name:literal) => {
        (
            $name,
            include_bytes!(concat!("../fixtures/", $name, ".fat.tzif")).as_slice(),
            include_bytes!(concat!("../fixtures/", $name, ".slim.tzif")).as_slice(),
            include_str!(concat!("../fixtures/", $name, ".zdump")),
        )
    };
}

const ZONES: &[(&str, &[u8], &[u8], &str)] = &[
    zone!("America_New_York"),
    zone!("Australia_Sydney"),
    zone!("Australia_Lord_Howe"),
    zone!("Europe_Dublin"),
    zone!("Europe_London"),
    zone!("Asia_Kolkata"),
    zone!("America_St_Johns"),
    zone!("Pacific_Chatham"),
    zone!("Africa_Casablanca"),
    zone!("America_Sao_Paulo"),
    zone!("Asia_Tehran"),
    zone!("Pacific_Apia"),
];

fn check(tz: &Tz, expected: &str, what: &str) -> usize {
    let mut checked = 0;
    for line in expected.lines() {
        let mut fields = line.split(' ');
        let t: i64 = fields.next().unwrap().parse().unwrap();
        let offset: i32 = fields.next().unwrap().parse().unwrap();
        let dst = fields.next().unwrap() == "1";
        let abbr = fields.next().unwrap();
        let got = tz.at(t);
        assert_eq!(
            (got.offset, got.dst, got.abbr.as_str()),
            (offset, dst, abbr),
            "{what} at {t}"
        );
        checked += 1;
    }
    checked
}

/// M0's `check-zones.py` as a test: every transition from 1900 to 2100 of
/// twelve zones chosen for their oddities, fat files and slim ones (whose
/// later transitions all come from the footer), matches `zdump`.
#[test]
fn every_transition_matches_zdump_fat_and_slim() {
    let mut total = 0;
    for &(name, fat, slim, expected) in ZONES {
        let fat = Tz::parse(fat).unwrap_or_else(|e| panic!("{name} fat: {e}"));
        let slim = Tz::parse(slim).unwrap_or_else(|e| panic!("{name} slim: {e}"));
        total += check(&fat, expected, &format!("{name} (fat)"));
        total += check(&slim, expected, &format!("{name} (slim)"));
    }
    // 4912 instants, each checked twice.
    assert_eq!(total, 9824);
}

/// The check can fail: London's rules are not New York's.
#[test]
fn a_wrong_zone_fails_the_check() {
    let london = Tz::parse(ZONES[4].2).unwrap();
    let new_york = ZONES[0].3;
    let caught = std::panic::catch_unwind(|| check(&london, new_york, "London as New York"));
    assert!(caught.is_err());
}

#[test]
fn every_prefix_of_a_file_is_refused_or_read_without_a_panic() {
    for &(name, fat, slim, _) in ZONES {
        for file in [fat, slim] {
            for len in 0..file.len() {
                if let Ok(tz) = Tz::parse(&file[..len]) {
                    for t in [i64::MIN, -1, 0, 1_790_000_000, i64::MAX] {
                        let _ = tz.at(t);
                    }
                }
            }
            assert!(Tz::parse(file).is_ok(), "{name}");
        }
    }
}

/// Random corruptions of real files: a property test standing in for the
/// fuzz target (`crates/scootbar/fuzz`) in every CI run.
#[test]
fn corrupted_files_never_panic() {
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut parsed = 0;
    for &(_, fat, slim, _) in ZONES {
        for file in [fat, slim] {
            let mut buf = file.to_vec();
            for _ in 0..2000 {
                buf.copy_from_slice(file);
                for _ in 0..1 + next() % 4 {
                    let at = (next() as usize) % buf.len();
                    buf[at] = next() as u8;
                }
                if let Ok(tz) = Tz::parse(&buf) {
                    parsed += 1;
                    for t in [i64::MIN, -(1 << 40), 0, 1_790_000_000, 1 << 40, i64::MAX] {
                        // Any offset a file or a footer can give is within
                        // a day and two hours.
                        assert!(tz.at(t).offset.abs() < 26 * 3600);
                    }
                }
            }
        }
    }
    assert!(parsed > 0, "no corruption parsed: the test checks nothing");
}

#[test]
fn a_file_over_the_cap_is_refused() {
    let big = vec![0u8; MAX_FILE + 1];
    assert_eq!(Tz::parse(&big), Err("too large"));
    assert_eq!(Tz::parse(b""), Err("truncated"));
    assert_eq!(Tz::parse(&[b'x'; 44]), Err("bad magic"));
}

/// A header claiming counts that cannot fit is refused before any
/// allocation sized by them.
#[test]
fn absurd_counts_are_refused() {
    let mut header = b"TZif2".to_vec();
    header.resize(44, 0);
    header[32..36].copy_from_slice(&u32::MAX.to_be_bytes()); // time count
    assert_eq!(Tz::parse(&header), Err("count too large"));
}

#[test]
fn v1_files_have_no_footer_and_keep_the_last_type() {
    // Transitions at 0 (to type 1) in a v1 file: two types, "AAA" and "BBB".
    let mut file = b"TZif\0".to_vec();
    file.resize(20, 0);
    for count in [0u32, 0, 0, 1, 2, 8] {
        file.extend_from_slice(&count.to_be_bytes());
    }
    file.extend_from_slice(&0i32.to_be_bytes());
    file.push(1);
    file.extend_from_slice(&3600i32.to_be_bytes());
    file.extend_from_slice(&[0, 0]);
    file.extend_from_slice(&7200i32.to_be_bytes());
    file.extend_from_slice(&[1, 4]);
    file.extend_from_slice(b"AAA\0BBB\0");
    let tz = Tz::parse(&file).unwrap();
    assert_eq!(tz.at(-1).offset, 3600);
    assert_eq!(tz.at(-1).abbr.as_str(), "AAA");
    assert_eq!(tz.at(0).offset, 7200);
    assert!(tz.at(i64::MAX).dst);
}

fn posix(text: &str) -> Tz {
    Tz::posix(text.as_bytes()).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn at(tz: &Tz, year: i64, month: u32, day: u32, hour: i64, minute: i64) -> Local {
    let t = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60;
    tz.at(t)
}

#[test]
fn a_posix_string_alone_is_a_zone() {
    let new_york = posix("EST5EDT,M3.2.0,M11.1.0");
    // 2026: DST from 8 March 07:00 UTC to 1 November 06:00 UTC.
    assert_eq!(at(&new_york, 2026, 3, 8, 6, 59).abbr.as_str(), "EST");
    assert_eq!(at(&new_york, 2026, 3, 8, 7, 0).abbr.as_str(), "EDT");
    assert_eq!(at(&new_york, 2026, 11, 1, 5, 59).offset, -4 * 3600);
    assert_eq!(at(&new_york, 2026, 11, 1, 6, 0).offset, -5 * 3600);
    let india = posix("IST-5:30");
    assert_eq!(india.at(0).offset, 5 * 3600 + 1800);
    let quoted = posix("<+0545>-5:45");
    assert_eq!(quoted.at(0).abbr.as_str(), "+0545");
    assert_eq!(quoted.at(0).offset, 5 * 3600 + 45 * 60);
}

/// `Jn` (February 29 never counted) and `n` (counted): no tzdata footer
/// uses either, so M0 could not check them against `zdump`.
#[test]
fn julian_dates_count_february_29_as_they_should() {
    // J60 is 1 March in every year; 59 (zero-based) is 1 March in a common
    // year and 29 February in a leap one. Summer time from that day, 02:00
    // standard, to day 300.
    let j = posix("AAA0BBB,J60,J300");
    let n = posix("AAA0BBB,59,299");
    // 2024 is a leap year.
    assert!(!at(&j, 2024, 2, 29, 3, 0).dst);
    assert!(at(&j, 2024, 3, 1, 3, 0).dst);
    assert!(at(&n, 2024, 2, 29, 3, 0).dst);
    assert!(!at(&n, 2024, 2, 28, 3, 0).dst);
    // 2025 is not: both start on 1 March.
    assert!(!at(&j, 2025, 2, 28, 3, 0).dst);
    assert!(at(&j, 2025, 3, 1, 3, 0).dst);
    assert!(at(&n, 2025, 3, 1, 3, 0).dst);
    assert!(!at(&n, 2025, 2, 28, 3, 0).dst);
}

#[test]
fn rule_times_may_be_negative_or_past_a_day() {
    // Greenland's footer shape: summer from the last Saturday of March at
    // -1:00 (23:00 the day before), to the last Saturday of October at 0.
    let greenland = posix("<-02>2<-01>,M3.5.0/-1,M10.5.0/0");
    // 2026: last Sunday of March is the 29th, so -1:00 is 28 March 23:00
    // local (-02), 29 March 01:00 UTC.
    assert!(!at(&greenland, 2026, 3, 29, 0, 59).dst);
    assert!(at(&greenland, 2026, 3, 29, 1, 0).dst);
    // Past 24 hours: /26 is 02:00 the next day.
    let late = posix("AAA0BBB,M3.2.0/26,M11.1.0");
    // 2026: second Sunday of March is the 8th; 26:00 is 9 March 02:00 UTC.
    assert!(!at(&late, 2026, 3, 9, 1, 59).dst);
    assert!(at(&late, 2026, 3, 9, 2, 0).dst);
}

#[test]
fn malformed_posix_strings_are_refused() {
    for bad in [
        "",
        "AB",
        "ABC",
        "EST5EDT",
        "EST5EDT,M3.2.0",
        "EST5EDT,M13.2.0,M11.1.0",
        "EST5EDT,M3.0.0,M11.1.0",
        "EST5EDT,M3.2.7,M11.1.0",
        "EST5EDT,J0,J100",
        "EST5EDT,J366,J100",
        "EST25",
        "EST5:60",
        "<+05>-5x",
        "<+0>-5",
        "<+05-5",
        "EST5EDT,M3.2.0/168,M11.1.0",
        "EST5EDT,M3.2.0,M11.1.0,",
        "EST1234",
    ] {
        assert!(Tz::posix(bad.as_bytes()).is_err(), "{bad:?} was accepted");
    }
}

#[test]
fn instants_past_the_clamp_do_not_overflow() {
    let new_york = Tz::parse(ZONES[0].2).unwrap();
    let sydney = posix("AEST-10AEDT,M10.1.0,M4.1.0/3");
    for tz in [&new_york, &sydney] {
        assert_eq!(tz.at(i64::MAX), tz.at(MAX_INSTANT));
        assert_eq!(tz.at(i64::MIN), tz.at(-MAX_INSTANT));
    }
}

#[test]
fn abbreviations_keep_only_what_prints() {
    assert_eq!(Abbr::new(b"EST").as_str(), "EST");
    assert_eq!(Abbr::new(b"+0530").as_str(), "+0530");
    assert_eq!(Abbr::new(b"A\x1b[2JB").as_str(), "A2JB");
    assert_eq!(Abbr::new(b"ABCDEFGHIJKL").as_str(), "ABCDEFGH");
    assert_eq!(Abbr::new(b"").as_str(), "");
    assert_eq!(Local::UTC.abbr.as_str(), "UTC");
}

#[test]
fn the_calendar_round_trips() {
    for days in (-800_000..800_000).step_by(997) {
        let (y, m, d) = civil_from_days(days);
        assert_eq!(days_from_civil(y, m, d), days);
    }
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    assert_eq!(weekday(0), 4); // a Thursday
    assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
    // The clamp's extremes stay in range.
    let far = MAX_INSTANT / 86_400;
    let (y, _, _) = civil_from_days(far);
    assert!(y > 35_000_000);
    assert!(days_from_civil(y, 1, 1) <= far);
}

#[test]
fn utc_is_utc() {
    let utc = Tz::utc();
    for t in [i64::MIN, 0, i64::MAX] {
        assert_eq!(utc.at(t), Local::UTC);
    }
}
