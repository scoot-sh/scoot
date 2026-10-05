//! The clock's format string: a small `strftime` subset, compiled once at
//! start-up and rendered on each tick with no allocation.
//!
//! The specifiers (the reference is `site/src/content/docs/scootbar/cli.md`):
//!
//! | | | | |
//! |---|---|---|---|
//! | `%H` hour, 00-23 | `%I` hour, 01-12 | `%k` hour, 0-23, space-padded | `%l` hour, 1-12, space-padded |
//! | `%M` minute | `%S` second | `%p` `AM`/`PM` | `%P` `am`/`pm` |
//! | `%a` `Mon` | `%A` `Monday` | `%b` (`%h`) `Sep` | `%B` `September` |
//! | `%d` day, 01-31 | `%e` day, space-padded | `%m` month, 01-12 | `%j` day of the year, 001-366 |
//! | `%y` year, 2 digits | `%Y` year | `%u` weekday, 1-7 from Monday | `%w` weekday, 0-6 from Sunday |
//! | `%Z` zone abbreviation | `%z` offset, `+hhmm` | `%R` `%H:%M` | `%T` `%H:%M:%S` |
//! | `%F` `%Y-%m-%d` | `%D` `%m/%d/%y` | `%%` a `%` | |
//!
//! A flag between `%` and the letter changes a number's padding, as in
//! GNU `date`: `-` none (`%-I` is `3`, not `03`), `_` spaces, `0` zeros.
//!
//! Names are English, as in the C locale: there is no locale lookup. An
//! unknown specifier, a `%` at the end, a control character (a newline or
//! a tab: the bar is one line), an empty format or one longer than
//! [`MAX_FORMAT`] bytes is refused when the flag is read, never at a tick.
//!
//! The default is `%-I:%M %P`: `3:07 pm`. A format that shows seconds
//! (`%S` or `%T`) makes the clock tick every second; any other ticks once a
//! minute.
//!
//! No dependencies but [`super::tzif`], so the fuzz target compiles this
//! file as it is.

use std::fmt::{self, Write};

use super::tzif::{self, Local};

#[cfg(test)]
mod tests;

/// The longest format accepted, in bytes.
pub const MAX_FORMAT: usize = 256;

/// The clock's default: 12-hour, no leading zero, lower-case `am`/`pm`.
/// [`Format::default`] builds it without parsing; a test holds the two
/// equal.
#[cfg(test)]
pub const DEFAULT: &str = "%-I:%M %P";

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// A local date and time, broken down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Civil {
    pub year: i64,
    /// 1 to 12.
    pub month: u32,
    /// 1 to 31.
    pub day: u32,
    /// 0 to 23.
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// 0 is Sunday.
    pub weekday: u32,
    /// 1 to 366.
    pub year_day: u32,
    pub local: Local,
}

impl Civil {
    /// The instant `t` (Unix seconds) in the local time type `local`.
    pub fn at(t: i64, local: Local) -> Self {
        let wall = tzif::clamp(t) + i64::from(local.offset);
        let days = wall.div_euclid(86_400);
        // 0 to 86399: fits.
        let second_of_day = wall.rem_euclid(86_400) as u32;
        let (year, month, day) = tzif::civil_from_days(days);
        let year_day = days - tzif::days_from_civil(year, 1, 1) + 1;
        Self {
            year,
            month,
            day,
            hour: second_of_day / 3600,
            minute: second_of_day / 60 % 60,
            second: second_of_day % 60,
            weekday: tzif::weekday(days),
            // 1 to 366.
            year_day: year_day as u32,
            local,
        }
    }
}

/// How a number is padded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pad {
    /// The specifier's own: zeros, or spaces for `%k`, `%l` and `%e`.
    Default,
    None,
    Space,
    Zero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Hour24,
    Hour12,
    /// `%k`: `%H` padded with a space.
    Hour24Space,
    /// `%l`: `%I` padded with a space.
    Hour12Space,
    Minute,
    Second,
    AmPm,
    AmPmLower,
    WeekdayShort,
    WeekdayLong,
    MonthShort,
    MonthLong,
    Day,
    /// `%e`: `%d` padded with a space.
    DaySpace,
    Month,
    YearDay,
    Year2,
    Year,
    /// `%u`: 1 to 7, Monday first.
    WeekdayMonday1,
    /// `%w`: 0 to 6, Sunday first.
    WeekdaySunday0,
    ZoneAbbr,
    ZoneOffset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    /// Bytes `start..end` of the format's literal text.
    Literal {
        start: u16,
        end: u16,
    },
    Field(Field, Pad),
}

/// A compiled format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Format {
    /// The literal text, in order; [`Item::Literal`] ranges index it.
    text: String,
    items: Vec<Item>,
    seconds: bool,
}

/// Why a format is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Empty,
    TooLong,
    /// `%` and a letter that is not a specifier (the character after `%`,
    /// and the flag, if any).
    Unknown(char),
    /// A `%` (and perhaps a flag) at the very end.
    Trailing,
    /// A control character: the bar is one line.
    Control,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "the format is empty"),
            Self::TooLong => write!(f, "a format is at most {MAX_FORMAT} bytes"),
            Self::Unknown(c) => write!(
                f,
                "`%{}` is not a specifier (see `scootbar daemon --help` for the list)",
                c.escape_debug()
            ),
            Self::Trailing => write!(f, "the format ends in a lone `%`"),
            Self::Control => write!(
                f,
                "the format has a control character (a newline or a tab); the bar is one line"
            ),
        }
    }
}

/// [`DEFAULT`], built without parsing, so having it cannot fail; a test
/// checks it is what parsing [`DEFAULT`] gives.
impl Default for Format {
    fn default() -> Self {
        Self {
            text: ": ".to_owned(),
            items: vec![
                Item::Field(Field::Hour12, Pad::None),
                Item::Literal { start: 0, end: 1 },
                Item::Field(Field::Minute, Pad::Default),
                Item::Literal { start: 1, end: 2 },
                Item::Field(Field::AmPmLower, Pad::Default),
            ],
            seconds: false,
        }
    }
}

impl Format {
    pub fn parse(format: &str) -> Result<Self, Error> {
        if format.is_empty() {
            return Err(Error::Empty);
        }
        if format.len() > MAX_FORMAT {
            return Err(Error::TooLong);
        }
        if format.chars().any(char::is_control) {
            return Err(Error::Control);
        }
        let mut compiled = Self {
            text: String::with_capacity(format.len()),
            items: Vec::new(),
            seconds: false,
        };
        let mut chars = format.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                compiled.literal(c);
                continue;
            }
            let mut spec = chars.next().ok_or(Error::Trailing)?;
            let pad = match spec {
                '-' => Pad::None,
                '_' => Pad::Space,
                '0' => Pad::Zero,
                _ => Pad::Default,
            };
            if pad != Pad::Default {
                spec = chars.next().ok_or(Error::Trailing)?;
            }
            compiled.specifier(spec, pad)?;
        }
        Ok(compiled)
    }

    /// Whether the format shows seconds, so the clock ticks every second.
    pub fn seconds(&self) -> bool {
        self.seconds
    }

    fn literal(&mut self, c: char) {
        let start = self.text.len();
        self.text.push(c);
        // At most `MAX_FORMAT` bytes of text: fits a `u16`.
        let end = self.text.len() as u16;
        if let Some(Item::Literal { end: last, .. }) = self.items.last_mut() {
            if usize::from(*last) == start {
                *last = end;
                return;
            }
        }
        self.items.push(Item::Literal {
            start: start as u16,
            end,
        });
    }

    fn field(&mut self, field: Field, pad: Pad) {
        if field == Field::Second {
            self.seconds = true;
        }
        self.items.push(Item::Field(field, pad));
    }

    fn specifier(&mut self, spec: char, pad: Pad) -> Result<(), Error> {
        use Field as F;
        let field = match spec {
            'H' => F::Hour24,
            'I' => F::Hour12,
            'k' => F::Hour24Space,
            'l' => F::Hour12Space,
            'M' => F::Minute,
            'S' => F::Second,
            'p' => F::AmPm,
            'P' => F::AmPmLower,
            'a' => F::WeekdayShort,
            'A' => F::WeekdayLong,
            'b' | 'h' => F::MonthShort,
            'B' => F::MonthLong,
            'd' => F::Day,
            'e' => F::DaySpace,
            'm' => F::Month,
            'j' => F::YearDay,
            'y' => F::Year2,
            'Y' => F::Year,
            'u' => F::WeekdayMonday1,
            'w' => F::WeekdaySunday0,
            'Z' => F::ZoneAbbr,
            'z' => F::ZoneOffset,
            '%' if pad == Pad::Default => {
                self.literal('%');
                return Ok(());
            }
            'R' | 'T' | 'F' | 'D' if pad == Pad::Default => {
                let fields: &[(Field, char)] = match spec {
                    'R' => &[(F::Hour24, ':'), (F::Minute, '\0')],
                    'T' => &[(F::Hour24, ':'), (F::Minute, ':'), (F::Second, '\0')],
                    'F' => &[(F::Year, '-'), (F::Month, '-'), (F::Day, '\0')],
                    _ => &[(F::Month, '/'), (F::Day, '/'), (F::Year2, '\0')],
                };
                for &(field, separator) in fields {
                    self.field(field, Pad::Default);
                    if separator != '\0' {
                        self.literal(separator);
                    }
                }
                return Ok(());
            }
            other => {
                return Err(Error::Unknown(other));
            }
        };
        self.field(field, pad);
        Ok(())
    }

    /// Writes `civil` in this format to `out`. Stops at the first error
    /// `out` returns (a full buffer), which is not an error here.
    pub fn render(&self, civil: &Civil, out: &mut impl Write) {
        for item in &self.items {
            let written = match *item {
                Item::Literal { start, end } => out.write_str(
                    self.text
                        .get(usize::from(start)..usize::from(end))
                        .unwrap_or(""),
                ),
                Item::Field(field, pad) => write_field(civil, field, pad, out),
            };
            if written.is_err() {
                return;
            }
        }
    }
}

fn write_field(civil: &Civil, field: Field, pad: Pad, out: &mut impl Write) -> fmt::Result {
    use Field as F;
    let hour12 = match civil.hour % 12 {
        0 => 12,
        h => h,
    };
    let pm = civil.hour >= 12;
    let (value, width, default) = match field {
        F::Hour24 => (i64::from(civil.hour), 2, Pad::Zero),
        F::Hour12 => (i64::from(hour12), 2, Pad::Zero),
        F::Hour24Space => (i64::from(civil.hour), 2, Pad::Space),
        F::Hour12Space => (i64::from(hour12), 2, Pad::Space),
        F::Minute => (i64::from(civil.minute), 2, Pad::Zero),
        F::Second => (i64::from(civil.second), 2, Pad::Zero),
        F::Day => (i64::from(civil.day), 2, Pad::Zero),
        F::DaySpace => (i64::from(civil.day), 2, Pad::Space),
        F::Month => (i64::from(civil.month), 2, Pad::Zero),
        F::YearDay => (i64::from(civil.year_day), 3, Pad::Zero),
        F::Year2 => (civil.year.rem_euclid(100), 2, Pad::Zero),
        // The year as it is, however many digits, as glibc.
        F::Year => (civil.year, 1, Pad::Zero),
        F::WeekdayMonday1 => (i64::from((civil.weekday + 6) % 7 + 1), 1, Pad::Zero),
        F::WeekdaySunday0 => (i64::from(civil.weekday), 1, Pad::Zero),
        F::AmPm => return out.write_str(if pm { "PM" } else { "AM" }),
        F::AmPmLower => return out.write_str(if pm { "pm" } else { "am" }),
        F::WeekdayShort => return out.write_str(short(name(&WEEKDAYS, civil.weekday))),
        F::WeekdayLong => return out.write_str(name(&WEEKDAYS, civil.weekday)),
        F::MonthShort => {
            return out.write_str(short(name(&MONTHS, civil.month.wrapping_sub(1))));
        }
        F::MonthLong => return out.write_str(name(&MONTHS, civil.month.wrapping_sub(1))),
        F::ZoneAbbr => return out.write_str(civil.local.abbr.as_str()),
        F::ZoneOffset => return write_offset(civil.local.offset, out),
    };
    let pad = match pad {
        Pad::Default => default,
        other => other,
    };
    match pad {
        Pad::None => write!(out, "{value}"),
        Pad::Space => write!(out, "{value:>width$}"),
        _ => write!(out, "{value:0width$}"),
    }
}

fn name(names: &[&'static str], index: u32) -> &'static str {
    usize::try_from(index)
        .ok()
        .and_then(|i| names.get(i))
        .copied()
        .unwrap_or("?")
}

/// The first three letters (every name is ASCII and at least three long).
fn short(name: &str) -> &str {
    name.get(..3).unwrap_or(name)
}

/// `+hhmm`, as `%z`: seconds of an offset are dropped, as glibc does.
fn write_offset(offset: i32, out: &mut impl Write) -> fmt::Result {
    let sign = if offset < 0 { '-' } else { '+' };
    let minutes = offset.unsigned_abs() / 60;
    write!(out, "{sign}{:02}{:02}", minutes / 60, minutes % 60)
}
