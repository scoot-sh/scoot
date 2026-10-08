//! A TZif reader (RFC 8536, versions 1 to 4) with the POSIX TZ footer,
//! sized for a clock: one lookup per tick. Ported from M0's spike
//! (`dev/spikes/scootbar/m0/clock/src/tzif.rs`), which matched `zdump` on
//! every transition of every zone in tzdata 2026c, fat and slim, from 1800
//! to 2200 (the record's §3b); the fixture tests here keep a slice of that
//! check in CI with no tzdata on the machine.
//!
//! Every read is bounds-checked and every malformed input is an `Err`,
//! never a panic: the input is a file anyone who can write `/etc/localtime`
//! or set `TZ` chooses. Past the spike:
//!
//! - **The instant is clamped** ([`MAX_INSTANT`]) before any arithmetic, so
//!   `t + offset` and the footer's day sums cannot overflow for a `t` near
//!   `i64::MAX` (the spike wrapped silently there).
//! - Nothing indexes: a lookup that cannot find its type falls back to the
//!   first type, which a parsed zone always has.
//! - Pure and free of I/O: `zone.rs` reads the file, capped at
//!   [`MAX_FILE`] while reading.
//!
//! No dependencies, so the fuzz target (`crates/scootbar/fuzz`) compiles
//! this file as it is.

#[cfg(test)]
mod tests;

/// A zone file bigger than this is refused (the largest in tzdata 2026c,
/// fat, is under 4 KiB; 64 KiB leaves room for any real one).
pub const MAX_FILE: usize = 64 * 1024;

/// The largest instant, either side of the epoch, the zone and the calendar
/// work with: about 35 million years. Instants past it are clamped to it;
/// no clock reads one, and the bound keeps every sum of seconds and days
/// far from overflowing an `i64`.
pub const MAX_INSTANT: i64 = 1 << 50;

/// Seconds in a day.
const DAY: i64 = 86_400;

/// A time zone abbreviation (`EST`, `+0530`): at most [`Abbr::CAP`] bytes,
/// only ASCII letters, digits, `+` and `-` (RFC 8536 §3.2 allows no more),
/// so it prints safely wherever it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Abbr {
    bytes: [u8; Self::CAP],
    len: u8,
}

impl Abbr {
    /// zic writes at most 6; room for a longer one from another tool.
    pub const CAP: usize = 8;

    /// The first [`Abbr::CAP`] bytes of `raw` that may be shown; others
    /// are dropped.
    pub fn new(raw: &[u8]) -> Self {
        let mut abbr = Self {
            bytes: [0; Self::CAP],
            len: 0,
        };
        for &byte in raw {
            if !(byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'-') {
                continue;
            }
            let Some(slot) = abbr.bytes.get_mut(usize::from(abbr.len)) else {
                break;
            };
            *slot = byte;
            abbr.len += 1;
        }
        abbr
    }

    pub fn as_str(&self) -> &str {
        let bytes = self.bytes.get(..usize::from(self.len)).unwrap_or(&[]);
        // ASCII only, by construction.
        std::str::from_utf8(bytes).unwrap_or("")
    }
}

/// A local time type: the offset from UTC in force, whether it is summer
/// time, and its abbreviation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local {
    /// Seconds east of UTC.
    pub offset: i32,
    pub dst: bool,
    pub abbr: Abbr,
}

impl Local {
    pub const UTC: Self = Self {
        offset: 0,
        dst: false,
        abbr: Abbr {
            bytes: *b"UTC\0\0\0\0\0",
            len: 3,
        },
    };
}

/// A time zone: TZif transitions and a footer rule, a footer rule alone (a
/// POSIX `TZ` string), or UTC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tz {
    /// Transition instants, strictly ascending.
    times: Vec<i64>,
    /// The type index in force from each transition on; as long as `times`,
    /// each below `types.len()`.
    index: Vec<u8>,
    /// Never empty.
    types: Vec<Local>,
    /// The rule for instants after the last transition (or all of them,
    /// with no transitions).
    footer: Option<Rule>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Date {
    /// `Jn`: 1 to 365, February 29 never counted.
    Julian1(u16),
    /// `n`: 0 to 365, February 29 counted.
    Julian0(u16),
    /// `Mm.w.d`: day `d` (0 is Sunday) of week `w` (5 is the last) of
    /// month `m`.
    Month { month: u8, week: u8, day: u8 },
}

/// A POSIX TZ rule: standard time, and summer time between two dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rule {
    std: Local,
    dst: Option<Summer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Summer {
    local: Local,
    start: Date,
    /// Seconds after local midnight (standard time) of `start`.
    start_time: i32,
    end: Date,
    /// Seconds after local midnight (summer time) of `end`.
    end_time: i32,
}

/// Why a zone could not be read.
pub type Error = &'static str;

struct Cursor<'a>(&'a [u8]);

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        if self.0.len() < n {
            return Err("truncated");
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }
}

fn be32(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?))
}

fn be64(bytes: &[u8]) -> Option<u64> {
    Some(u64::from_be_bytes(bytes.get(..8)?.try_into().ok()?))
}

/// The six counts of a TZif header, in file order.
#[derive(Debug, Clone, Copy)]
struct Counts {
    isut: usize,
    isstd: usize,
    leap: usize,
    time: usize,
    types: usize,
    chars: usize,
}

impl Counts {
    /// Bytes of the data block the counts describe, with `time_size`-byte
    /// times. Each count is at most [`MAX_FILE`], so this cannot overflow.
    fn block(self, time_size: usize) -> usize {
        self.time * (time_size + 1)
            + self.types * 6
            + self.chars
            + self.leap * (time_size + 4)
            + self.isstd
            + self.isut
    }
}

impl Tz {
    /// Parses a TZif file.
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        if data.len() > MAX_FILE {
            return Err("too large");
        }
        let mut cursor = Cursor(data);
        let (version, v1_counts) = header(&mut cursor)?;
        let (counts, time_size) = if version == 0 {
            (v1_counts, 4)
        } else {
            // Skip the 32-bit block, then read the 64-bit one.
            cursor.take(v1_counts.block(4))?;
            (header(&mut cursor)?.1, 8)
        };
        if counts.types == 0 || counts.types > 256 || counts.chars == 0 {
            return Err("bad counts");
        }
        let raw_times = cursor.take(counts.time * time_size)?;
        let mut times = Vec::with_capacity(counts.time);
        for chunk in raw_times.chunks_exact(time_size) {
            let time = if time_size == 4 {
                i64::from(be32(chunk).ok_or("truncated")? as i32)
            } else {
                be64(chunk).ok_or("truncated")? as i64
            };
            if times.last().is_some_and(|&last| last >= time) {
                return Err("transitions not ascending");
            }
            times.push(time);
        }
        let index = cursor.take(counts.time)?.to_vec();
        if index.iter().any(|&i| usize::from(i) >= counts.types) {
            return Err("type index out of range");
        }
        let raw_types = cursor.take(counts.types * 6)?;
        let chars = cursor.take(counts.chars)?;
        let mut types = Vec::with_capacity(counts.types);
        for raw in raw_types.chunks_exact(6) {
            let offset = be32(raw).ok_or("truncated")? as i32;
            // RFC 8536 §3.2: -2^31 is not allowed, and zic keeps offsets
            // within a day either way.
            if !(-89_999..=93_599).contains(&offset) {
                return Err("offset out of range");
            }
            let dst = *raw.get(4).ok_or("truncated")? != 0;
            let at = usize::from(*raw.get(5).ok_or("truncated")?);
            let rest = chars.get(at..).ok_or("abbreviation index out of range")?;
            let end = rest
                .iter()
                .position(|&b| b == 0)
                .ok_or("abbreviation not terminated")?;
            types.push(Local {
                offset,
                dst,
                abbr: Abbr::new(rest.get(..end).unwrap_or(&[])),
            });
        }
        cursor.take(counts.leap * (time_size + 4))?;
        cursor.take(counts.isstd + counts.isut)?;
        let footer = if version == 0 {
            None
        } else {
            let rest = cursor.0;
            let body = rest.strip_prefix(b"\n").ok_or("footer missing")?;
            let end = body
                .iter()
                .position(|&b| b == b'\n')
                .ok_or("footer unterminated")?;
            let text = body.get(..end).unwrap_or(&[]);
            // An empty footer means "no rule"; an unparsable one is taken
            // the same way, so the last transition's type stays in force.
            if text.is_empty() {
                None
            } else {
                parse_rule(text).ok()
            }
        };
        Ok(Self {
            times,
            index,
            types,
            footer,
        })
    }

    /// A zone from a POSIX TZ string alone (`TZ=EST5EDT,M3.2.0,M11.1.0`),
    /// as glibc reads `TZ` when it names no zone file. A string naming
    /// summer time but no rule (`TZ=EST5EDT`) takes glibc's default US
    /// rules (March's second Sunday to November's first, at 02:00), as
    /// `date` shows with no zoneinfo installed; anything else unparsable
    /// is refused.
    pub fn posix(text: &[u8]) -> Result<Self, Error> {
        let rule = parse_rule(text).or_else(|error| {
            if error == "summer time without a rule" {
                let mut defaulted = text.to_vec();
                defaulted.extend_from_slice(b",M3.2.0,M11.1.0");
                parse_rule(&defaulted)
            } else {
                Err(error)
            }
        })?;
        Ok(Self {
            times: Vec::new(),
            index: Vec::new(),
            types: vec![rule.std],
            footer: Some(rule),
        })
    }

    /// UTC, for when there is no usable zone at all.
    pub fn utc() -> Self {
        Self {
            times: Vec::new(),
            index: Vec::new(),
            types: vec![Local::UTC],
            footer: None,
        }
    }

    /// The local time type in force at `t` (Unix seconds).
    pub fn at(&self, t: i64) -> Local {
        let t = clamp(t);
        let first = self.types.first().copied().unwrap_or(Local::UTC);
        let by_index = |i: Option<&u8>| {
            i.and_then(|&i| self.types.get(usize::from(i)))
                .copied()
                .unwrap_or(first)
        };
        match self.times.last() {
            Some(&last) if t >= last => match &self.footer {
                Some(rule) => rule.at(t),
                None => by_index(self.index.last()),
            },
            Some(_) => match self.times.partition_point(|&x| x <= t) {
                // Before the first transition: type 0 (RFC 8536 §3.2).
                0 => first,
                i => by_index(self.index.get(i - 1)),
            },
            None => match &self.footer {
                Some(rule) => rule.at(t),
                None => first,
            },
        }
    }
}

/// `t` within ±[`MAX_INSTANT`].
pub fn clamp(t: i64) -> i64 {
    t.clamp(-MAX_INSTANT, MAX_INSTANT)
}

fn header(cursor: &mut Cursor) -> Result<(u8, Counts), Error> {
    let head = cursor.take(44)?;
    if head.get(..4) != Some(b"TZif") {
        return Err("bad magic");
    }
    let version = match head.get(4) {
        Some(0) => 0,
        Some(&v @ b'2'..=b'4') => v - b'0',
        _ => return Err("bad version"),
    };
    let mut values = [0usize; 6];
    for (i, value) in values.iter_mut().enumerate() {
        let raw = be32(head.get(20 + i * 4..).unwrap_or(&[])).ok_or("truncated")?;
        *value = usize::try_from(raw).map_err(|_| "count too large")?;
        if *value > MAX_FILE {
            return Err("count too large");
        }
    }
    let [isut, isstd, leap, time, types, chars] = values;
    Ok((
        version,
        Counts {
            isut,
            isstd,
            leap,
            time,
            types,
            chars,
        },
    ))
}

// ---- POSIX TZ strings (the footer), with RFC 8536's extensions ----

fn parse_rule(text: &[u8]) -> Result<Rule, Error> {
    let mut p = text;
    let std_name = name(&mut p)?;
    let std_offset = -offset(&mut p, 24)?;
    let std = Local {
        offset: std_offset,
        dst: false,
        abbr: Abbr::new(std_name),
    };
    if p.is_empty() {
        return Ok(Rule { std, dst: None });
    }
    let dst_name = name(&mut p)?;
    let dst_offset = if p.first().is_some_and(|&b| b != b',') {
        -offset(&mut p, 24)?
    } else {
        std_offset + 3600
    };
    let local = Local {
        offset: dst_offset,
        dst: true,
        abbr: Abbr::new(dst_name),
    };
    // Summer time with no rule: zic never writes one in a footer, and
    // POSIX leaves the dates to the implementation, so a footer is refused
    // rather than guessed; [`Tz::posix`] retries those with glibc's
    // default US rules, which is what `TZ` without a zone file means.
    p = p.strip_prefix(b",").ok_or("summer time without a rule")?;
    let (start, start_time) = date_time(&mut p)?;
    p = p.strip_prefix(b",").ok_or("missing end rule")?;
    let (end, end_time) = date_time(&mut p)?;
    if !p.is_empty() {
        return Err("trailing footer bytes");
    }
    Ok(Rule {
        std,
        dst: Some(Summer {
            local,
            start,
            start_time,
            end,
            end_time,
        }),
    })
}

fn name<'a>(p: &mut &'a [u8]) -> Result<&'a [u8], Error> {
    if let Some(rest) = p.strip_prefix(b"<") {
        let end = rest
            .iter()
            .position(|&b| b == b'>')
            .ok_or("unterminated <name>")?;
        let (name, after) = rest.split_at(end);
        if name.len() < 3
            || !name
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b'+' || *b == b'-')
        {
            return Err("bad quoted name");
        }
        *p = after.get(1..).unwrap_or(&[]);
        Ok(name)
    } else {
        let end = p
            .iter()
            .position(|b| !b.is_ascii_alphabetic())
            .unwrap_or(p.len());
        if end < 3 {
            return Err("short name");
        }
        let (name, after) = p.split_at(end);
        *p = after;
        Ok(name)
    }
}

/// One to three digits, at most `max`.
fn number(p: &mut &[u8], max: i32) -> Result<i32, Error> {
    let end = p
        .iter()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(p.len());
    if end == 0 || end > 3 {
        return Err("bad number");
    }
    let (digits, rest) = p.split_at(end);
    let value = digits
        .iter()
        .fold(0i32, |v, &b| v * 10 + i32::from(b - b'0'));
    *p = rest;
    if value > max {
        Err("number out of range")
    } else {
        Ok(value)
    }
}

/// `[+-]hh[:mm[:ss]]` in seconds, POSIX sign (positive is west of UTC).
fn offset(p: &mut &[u8], max_hours: i32) -> Result<i32, Error> {
    let negative = match p.first() {
        Some(b'-') => true,
        Some(b'+') => false,
        _ => return hms(p, max_hours),
    };
    *p = p.get(1..).unwrap_or(&[]);
    let value = hms(p, max_hours)?;
    Ok(if negative { -value } else { value })
}

fn hms(p: &mut &[u8], max_hours: i32) -> Result<i32, Error> {
    let mut value = number(p, max_hours)? * 3600;
    if let Some(rest) = p.strip_prefix(b":") {
        *p = rest;
        value += number(p, 59)? * 60;
        if let Some(rest) = p.strip_prefix(b":") {
            *p = rest;
            value += number(p, 59)?;
        }
    }
    Ok(value)
}

fn date_time(p: &mut &[u8]) -> Result<(Date, i32), Error> {
    let date = if let Some(rest) = p.strip_prefix(b"J") {
        *p = rest;
        let n = number(p, 365)?;
        if n == 0 {
            return Err("J0");
        }
        Date::Julian1(n as u16)
    } else if let Some(rest) = p.strip_prefix(b"M") {
        *p = rest;
        let month = number(p, 12)?;
        *p = p.strip_prefix(b".").ok_or("bad M rule")?;
        let week = number(p, 5)?;
        *p = p.strip_prefix(b".").ok_or("bad M rule")?;
        let day = number(p, 6)?;
        if month == 0 || week == 0 {
            return Err("bad M rule");
        }
        // Each at most 12, 5 and 6: they fit.
        Date::Month {
            month: month as u8,
            week: week as u8,
            day: day as u8,
        }
    } else {
        Date::Julian0(number(p, 365)? as u16)
    };
    // RFC 8536 §3.3.1: the time may be negative and up to 167 hours.
    let time = match p.strip_prefix(b"/") {
        Some(rest) => {
            *p = rest;
            offset(p, 167)?
        }
        None => 7200,
    };
    Ok((date, time))
}

impl Rule {
    /// `t` already clamped.
    fn at(&self, t: i64) -> Local {
        let Some(summer) = self.dst else {
            return self.std;
        };
        // The year of `t` in local standard time.
        let year = civil_from_days((t + i64::from(self.std.offset)).div_euclid(DAY)).0;
        let start = day_of(year, summer.start) * DAY + i64::from(summer.start_time)
            - i64::from(self.std.offset);
        let end = day_of(year, summer.end) * DAY + i64::from(summer.end_time)
            - i64::from(summer.local.offset);
        let in_summer = if start <= end {
            start <= t && t < end
        } else {
            !(end <= t && t < start)
        };
        if in_summer { summer.local } else { self.std }
    }
}

fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn day_of(year: i64, date: Date) -> i64 {
    let jan1 = days_from_civil(year, 1, 1);
    match date {
        Date::Julian1(n) => {
            let n = i64::from(n) - 1;
            jan1 + n + i64::from(is_leap(year) && n >= 59)
        }
        Date::Julian0(n) => jan1 + i64::from(n),
        Date::Month { month, week, day } => {
            let first = days_from_civil(year, u32::from(month), 1);
            let weekday_of_first = weekday(first);
            let mut offset = (i64::from(day) - i64::from(weekday_of_first)).rem_euclid(7)
                + (i64::from(week) - 1) * 7;
            let length = days_in_month(year, u32::from(month));
            // Week 5 means the last: at most one week back.
            while offset >= length {
                offset -= 7;
            }
            first + offset
        }
    }
}

pub fn days_in_month(year: i64, month: u32) -> i64 {
    match month {
        2 => 28 + i64::from(is_leap(year)),
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The weekday of a day counted from 1970-01-01 (a Thursday): 0 is Sunday.
pub fn weekday(days: i64) -> u32 {
    // 0 to 6.
    (days + 4).rem_euclid(7) as u32
}

/// Howard Hinnant's `days_from_civil` (public domain): days from
/// 1970-01-01 to a proleptic Gregorian date.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = (i64::from(month) + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Howard Hinnant's `civil_from_days` (public domain): (year, month 1 to
/// 12, day 1 to 31).
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    // 1 to 31 and 1 to 12: they fit.
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}
