//! UTC calendar arithmetic for the sun module's callers: Unix seconds to
//! and from a proleptic Gregorian date, and the spelling systemd's
//! `OnCalendar=` takes. No time crate and no `date` process: the civil
//! calendar is Howard Hinnant's days-from-civil algorithm, exact for every
//! date from year 0000 to 9999.

/// Seconds since the epoch of a UTC date and time. Fields are taken as
/// given (a caller that wants validation checks the ranges first).
pub fn unix_from_utc(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + hour * 3_600 + minute * 60 + second
}

/// `(year, month, day, hour, minute, second)` of a Unix time in UTC.
pub fn utc_from_unix(unix: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = unix.div_euclid(86_400);
    let rest = unix.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, rest / 3_600, rest % 3_600 / 60, rest % 60)
}

/// The `OnCalendar=` spelling of one instant: `2026-11-08 11:52:03 UTC`.
/// The `UTC` suffix makes the manager read it as UTC whatever its zone is.
/// Years outside 0000 to 9999 are not a date systemd takes, so they are
/// `None`.
pub fn on_calendar_utc(unix: i64) -> Option<String> {
    let (year, month, day, hour, minute, second) = utc_from_unix(unix);
    (0..=9999)
        .contains(&year)
        .then(|| format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC"))
}
