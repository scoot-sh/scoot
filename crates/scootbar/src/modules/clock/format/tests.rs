use super::{Civil, DEFAULT, Error, Format, MAX_FORMAT};
use crate::modules::clock::tzif::{Abbr, Local, days_from_civil};

fn local(offset: i32, abbr: &str) -> Local {
    Local {
        offset,
        dst: false,
        abbr: Abbr::new(abbr.as_bytes()),
    }
}

/// `year-month-day hour:minute:second` UTC, shown in `local`.
fn civil(date: (i64, u32, u32), time: (i64, i64, i64), at: Local) -> Civil {
    let t = days_from_civil(date.0, date.1, date.2) * 86_400 + time.0 * 3600 + time.1 * 60 + time.2;
    Civil::at(t - i64::from(at.offset), at)
}

fn show(format: &str, civil: &Civil) -> String {
    let mut out = String::new();
    Format::parse(format)
        .unwrap_or_else(|e| panic!("{format}: {e}"))
        .render(civil, &mut out);
    out
}

#[test]
fn the_default_is_twelve_hour_lower_case_with_no_leading_zero() {
    let utc = local(0, "UTC");
    assert_eq!(Format::default(), Format::parse(DEFAULT).unwrap());
    let cases = [
        ((15, 7, 0), "3:07 pm"),
        ((0, 0, 0), "12:00 am"),
        ((12, 0, 0), "12:00 pm"),
        ((0, 59, 59), "12:59 am"),
        ((9, 5, 0), "9:05 am"),
        ((11, 59, 0), "11:59 am"),
        ((23, 59, 59), "11:59 pm"),
        ((13, 0, 0), "1:00 pm"),
    ];
    for (time, expected) in cases {
        let at = civil((2026, 9, 29), time, utc);
        assert_eq!(show(DEFAULT, &at), expected, "{time:?}");
        let mut out = String::new();
        Format::default().render(&at, &mut out);
        assert_eq!(out, expected);
    }
    assert!(!Format::default().seconds());
}

#[test]
fn twenty_four_hours_is_one_format_away() {
    let at = civil((2026, 9, 29), (15, 7, 0), local(0, "UTC"));
    assert_eq!(show("%H:%M", &at), "15:07");
    assert_eq!(show("%R", &at), "15:07");
    let early = civil((2026, 9, 29), (3, 7, 9), local(0, "UTC"));
    assert_eq!(show("%H:%M", &early), "03:07");
    assert_eq!(show("%k:%M", &early), " 3:07");
    assert_eq!(show("%-H:%M", &early), "3:07");
    assert_eq!(show("%T", &early), "03:07:09");
}

#[test]
fn every_specifier() {
    // Tuesday 29 September 2026, 15:07:09, at -04:00 (EDT).
    let at = civil((2026, 9, 29), (15, 7, 9), local(-4 * 3600, "EDT"));
    for (format, expected) in [
        ("%H", "15"),
        ("%I", "03"),
        ("%k", "15"),
        ("%l", " 3"),
        ("%M", "07"),
        ("%S", "09"),
        ("%p", "PM"),
        ("%P", "pm"),
        ("%a", "Tue"),
        ("%A", "Tuesday"),
        ("%b", "Sep"),
        ("%h", "Sep"),
        ("%B", "September"),
        ("%d", "29"),
        ("%e", "29"),
        ("%m", "09"),
        ("%j", "272"),
        ("%y", "26"),
        ("%Y", "2026"),
        ("%u", "2"),
        ("%w", "2"),
        ("%Z", "EDT"),
        ("%z", "-0400"),
        ("%R", "15:07"),
        ("%T", "15:07:09"),
        ("%F", "2026-09-29"),
        ("%D", "09/29/26"),
        ("%%", "%"),
        ("%a %d %b %H:%M", "Tue 29 Sep 15:07"),
        ("🕒 %-I:%M", "🕒 3:07"),
    ] {
        assert_eq!(show(format, &at), expected, "{format}");
    }
}

#[test]
fn padding_flags() {
    let at = civil((2026, 1, 5), (7, 3, 4), local(0, "UTC"));
    for (format, expected) in [
        ("%d", "05"),
        ("%-d", "5"),
        ("%_d", " 5"),
        ("%e", " 5"),
        ("%-e", "5"),
        ("%0e", "05"),
        ("%j", "005"),
        ("%-j", "5"),
        ("%_j", "  5"),
        ("%-m", "1"),
        ("%-M", "3"),
        ("%-S", "4"),
        ("%_H", " 7"),
        ("%0k", "07"),
        ("%-p", "AM"),
    ] {
        assert_eq!(show(format, &at), expected, "{format}");
    }
}

#[test]
fn edges_of_the_calendar() {
    let utc = local(0, "UTC");
    // A leap day and the year's last day.
    let leap = civil((2024, 12, 31), (0, 0, 0), utc);
    assert_eq!(show("%j %u %w %a", &leap), "366 2 2 Tue");
    // Sunday is 7 for %u, 0 for %w.
    let sunday = civil((2026, 10, 4), (0, 0, 0), utc);
    assert_eq!(show("%u %w %A", &sunday), "7 0 Sunday");
    // Years before the epoch, and before year 1.
    let old = civil((1900, 3, 1), (12, 0, 0), utc);
    assert_eq!(show("%Y %y", &old), "1900 00");
    let bc = Civil::at(days_from_civil(-5, 6, 1) * 86_400, utc);
    assert_eq!(show("%Y %y", &bc), "-5 95");
    // Offsets with minutes, and the far end of the clamp.
    let india = civil((2026, 9, 29), (15, 7, 0), local(5 * 3600 + 1800, "IST"));
    assert_eq!(show("%z %Z", &india), "+0530 IST");
    let newfoundland = civil((2026, 9, 29), (15, 7, 0), local(-(3 * 3600 + 1800), "NDT"));
    assert_eq!(show("%z", &newfoundland), "-0330");
    let far = Civil::at(i64::MAX, utc);
    assert!(show("%Y-%m-%d %H:%M:%S", &far).starts_with("35"));
}

#[test]
fn seconds_decide_the_tick() {
    assert!(!Format::parse("%H:%M").unwrap().seconds());
    assert!(Format::parse("%H:%M:%S").unwrap().seconds());
    assert!(Format::parse("%T").unwrap().seconds());
    assert!(Format::parse("%-S").unwrap().seconds());
    assert!(!Format::parse("%R %%S").unwrap().seconds());
}

#[test]
fn bad_formats_are_refused_with_a_reason() {
    assert_eq!(Format::parse(""), Err(Error::Empty));
    assert_eq!(Format::parse("%"), Err(Error::Trailing));
    assert_eq!(Format::parse("%-"), Err(Error::Trailing));
    assert_eq!(Format::parse("%Q"), Err(Error::Unknown('Q')));
    assert_eq!(Format::parse("%E"), Err(Error::Unknown('E')));
    assert_eq!(Format::parse("%-%"), Err(Error::Unknown('%')));
    assert_eq!(Format::parse("%-R"), Err(Error::Unknown('R')));
    assert_eq!(Format::parse("%\u{e9}"), Err(Error::Unknown('\u{e9}')));
    assert_eq!(Format::parse("a\nb"), Err(Error::Control));
    assert_eq!(Format::parse("\t%H"), Err(Error::Control));
    assert_eq!(Format::parse("\u{7f}"), Err(Error::Control));
    assert_eq!(
        Format::parse(&"x".repeat(MAX_FORMAT + 1)),
        Err(Error::TooLong)
    );
    assert!(Format::parse(&"x".repeat(MAX_FORMAT)).is_ok());
    let message = Error::Unknown('\u{1b}').to_string();
    assert!(!message.contains('\u{1b}'), "{message:?}");
}

/// A writer that stops after `room` bytes, as the bar's view text does.
struct Tight {
    text: String,
    room: usize,
}

impl std::fmt::Write for Tight {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        if self.text.len() + s.len() > self.room {
            return Err(std::fmt::Error);
        }
        self.text.push_str(s);
        Ok(())
    }
}

#[test]
fn a_full_buffer_stops_the_render_cleanly() {
    let at = civil((2026, 9, 29), (15, 7, 9), local(0, "UTC"));
    let format = Format::parse("%A %B %d").unwrap();
    let mut out = Tight {
        text: String::new(),
        room: 10,
    };
    format.render(&at, &mut out);
    assert_eq!(out.text, "Tuesday ");
}

/// The property test over the format string (the fuzz target in
/// `crates/scootbar/fuzz` runs the same property without end): any string
/// either parses or is refused, never panics; one that parses renders any
/// instant without panicking, into at most a bounded length, and parses the
/// same way every time.
#[test]
fn any_format_string_parses_or_is_refused_and_renders_bounded() {
    const ALPHABET: &[char] = &[
        '%', '%', '%', '-', '_', '0', 'H', 'I', 'k', 'l', 'M', 'S', 'p', 'P', 'a', 'A', 'b', 'h',
        'B', 'd', 'e', 'm', 'j', 'y', 'Y', 'u', 'w', 'Z', 'z', 'R', 'T', 'F', 'D', 'Q', 'c', ':',
        ' ', 'x', '\n', 'é', '🕒', '\u{0}',
    ];
    let mut x = 0x2545_f491_4f6c_dd1du64;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let instants = [
        i64::MIN,
        -(1 << 40),
        -1,
        0,
        1_790_651_220,
        1 << 40,
        i64::MAX,
    ];
    let zones = [
        local(0, "UTC"),
        local(93_599, "+2559"),
        local(-89_999, "-2459"),
    ];
    let (mut parsed, mut refused) = (0, 0);
    for _ in 0..20_000 {
        let len = (next() % 24) as usize;
        let text: String = (0..len)
            .map(|_| ALPHABET[(next() % ALPHABET.len() as u64) as usize])
            .collect();
        match Format::parse(&text) {
            Ok(format) => {
                parsed += 1;
                assert_eq!(Format::parse(&text).as_ref(), Ok(&format));
                for &t in &instants {
                    for &zone in &zones {
                        let mut out = String::new();
                        format.render(&Civil::at(t, zone), &mut out);
                        // No specifier writes more than 9 bytes but a
                        // year, at most 9 digits after the clamp.
                        assert!(out.len() <= text.len() * 10 + 16, "{text:?}: {out:?}");
                        assert!(!out.chars().any(char::is_control), "{text:?}: {out:?}");
                    }
                }
            }
            Err(_) => refused += 1,
        }
    }
    assert!(
        parsed > 1000 && refused > 1000,
        "{parsed} parsed, {refused} refused"
    );
}
