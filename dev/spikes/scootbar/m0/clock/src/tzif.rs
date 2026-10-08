//! A hand-rolled TZif (RFC 8536, versions 1-4) reader with the POSIX TZ
//! footer, sized for a clock: one lookup per minute. Spike quality, but
//! every read is bounds-checked and every malformed input is an `Err`.

/// A zone file bigger than this is refused (the largest in tzdata 2026c,
/// fat, is under 4 KiB; 64 KiB leaves room for any real one).
pub const MAX_FILE: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Local {
    /// Seconds east of UTC.
    pub utoff: i32,
    pub isdst: bool,
}

#[derive(Debug, Clone, Copy)]
struct Type {
    utoff: i32,
    isdst: bool,
    abbr: [u8; 8],
}

#[derive(Debug)]
pub struct Tz {
    times: Vec<i64>,
    idx: Vec<u8>,
    types: Vec<Type>,
    footer: Option<Rule>,
}

#[derive(Debug, Clone, Copy)]
enum Date {
    /// Jn: 1..=365, Feb 29 never counted.
    Julian1(u16),
    /// n: 0..=365, Feb 29 counted.
    Julian0(u16),
    /// Mm.w.d
    Month { m: u8, w: u8, d: u8 },
}

#[derive(Debug, Clone, Copy)]
struct Rule {
    std: Type,
    dst: Option<(Type, Date, i32, Date, i32)>,
}

struct Cur<'a>(&'a [u8]);
impl<'a> Cur<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], &'static str> {
        if self.0.len() < n {
            return Err("truncated");
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
}

fn abbr(s: &[u8]) -> [u8; 8] {
    let mut a = [0u8; 8];
    for (d, &c) in a.iter_mut().zip(s.iter().take(7)) {
        *d = c;
    }
    a
}

impl Tz {
    pub fn parse(data: &[u8]) -> Result<Tz, &'static str> {
        if data.len() > MAX_FILE {
            return Err("too large");
        }
        let mut c = Cur(data);
        let (ver, counts) = header(&mut c)?;
        let v1 = ver == 0;
        if !v1 {
            // Skip the 32-bit block, then read the 64-bit one.
            let [isut, isstd, leap, time, typ, chr] = counts;
            let skip = time * 5 + typ * 6 + chr + leap * 8 + isstd + isut;
            c.take(skip)?;
        }
        let counts = if v1 { counts } else { header(&mut c)?.1 };
        let [isut, isstd, leap, time, typ, chr] = counts;
        if typ == 0 || typ > 256 || chr == 0 {
            return Err("bad counts");
        }
        let tsize = if v1 { 4 } else { 8 };
        let tbytes = c.take(time * tsize)?;
        let times: Vec<i64> = tbytes
            .chunks_exact(tsize)
            .map(|b| {
                if v1 {
                    i64::from(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
                } else {
                    i64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
                }
            })
            .collect();
        if times.windows(2).any(|w| w[0] >= w[1]) {
            return Err("transitions not ascending");
        }
        let idx = c.take(time)?.to_vec();
        if idx.iter().any(|&i| usize::from(i) >= typ) {
            return Err("type index out of range");
        }
        let traw = c.take(typ * 6)?;
        let chars = c.take(chr)?;
        let mut types = Vec::with_capacity(typ);
        for t in traw.chunks_exact(6) {
            let utoff = i32::from_be_bytes([t[0], t[1], t[2], t[3]]);
            if utoff == i32::MIN || !(-89999..=93599).contains(&utoff) {
                return Err("utoff out of range");
            }
            let di = usize::from(t[5]);
            let rest = chars.get(di..).ok_or("desig index out of range")?;
            let end = rest
                .iter()
                .position(|&b| b == 0)
                .ok_or("desig not terminated")?;
            types.push(Type {
                utoff,
                isdst: t[4] != 0,
                abbr: abbr(&rest[..end]),
            });
        }
        c.take(leap * (tsize + 4))?;
        c.take(isstd + isut)?;
        let footer = if v1 {
            None
        } else {
            let f = c.0;
            if f.first() != Some(&b'\n') {
                return Err("footer missing");
            }
            let end = f[1..]
                .iter()
                .position(|&b| b == b'\n')
                .ok_or("footer unterminated")?;
            let s = &f[1..1 + end];
            // An empty footer means "no rule"; an unparsable one is treated
            // the same, so the last transition's type stays in force.
            if s.is_empty() {
                None
            } else {
                parse_rule(s).ok()
            }
        };
        Ok(Tz {
            times,
            idx,
            types,
            footer,
        })
    }

    /// UTC, for when there is no usable zone file at all.
    pub fn utc() -> Tz {
        Tz {
            times: Vec::new(),
            idx: Vec::new(),
            types: vec![Type {
                utoff: 0,
                isdst: false,
                abbr: abbr(b"UTC"),
            }],
            footer: None,
        }
    }

    /// The local time type in force at `t` (Unix seconds), and its abbreviation.
    pub fn at(&self, t: i64) -> (Local, [u8; 8]) {
        let ty = match self.times.last() {
            Some(&last) if t >= last => match &self.footer {
                Some(r) => r.at(t),
                None => self.types[usize::from(self.idx[self.idx.len() - 1])],
            },
            None => match &self.footer {
                Some(r) => r.at(t),
                None => self.types[0],
            },
            Some(_) => {
                let i = self.times.partition_point(|&x| x <= t);
                if i == 0 {
                    self.types[0]
                } else {
                    self.types[usize::from(self.idx[i - 1])]
                }
            }
        };
        (
            Local {
                utoff: ty.utoff,
                isdst: ty.isdst,
            },
            ty.abbr,
        )
    }
}

fn header(c: &mut Cur) -> Result<(u8, [usize; 6]), &'static str> {
    let h = c.take(44)?;
    if &h[..4] != b"TZif" {
        return Err("bad magic");
    }
    let ver = match h[4] {
        0 => 0,
        b'2'..=b'4' => h[4] - b'0',
        _ => return Err("bad version"),
    };
    let mut n = [0usize; 6];
    for (i, v) in n.iter_mut().enumerate() {
        let o = 20 + i * 4;
        *v = u32::from_be_bytes([h[o], h[o + 1], h[o + 2], h[o + 3]]) as usize;
        if *v > MAX_FILE {
            return Err("count too large");
        }
    }
    Ok((ver, n))
}

// ---- POSIX TZ strings (the footer), with RFC 8536's extensions ----

fn parse_rule(s: &[u8]) -> Result<Rule, &'static str> {
    let mut p = s;
    let std_name = name(&mut p)?;
    let std_off = -offset(&mut p, 24)?;
    let std = Type {
        utoff: std_off,
        isdst: false,
        abbr: abbr(std_name),
    };
    if p.is_empty() {
        return Ok(Rule { std, dst: None });
    }
    let dst_name = name(&mut p)?;
    let dst_off = if p.first().is_some_and(|&b| b != b',') {
        -offset(&mut p, 24)?
    } else {
        std_off + 3600
    };
    let dst = Type {
        utoff: dst_off,
        isdst: true,
        abbr: abbr(dst_name),
    };
    // A footer with DST but no rule is not something zic writes.
    let rest = p.strip_prefix(b",").ok_or("dst without rule")?;
    p = rest;
    let (d1, t1) = date_time(&mut p)?;
    p = p.strip_prefix(b",").ok_or("missing end rule")?;
    let (d2, t2) = date_time(&mut p)?;
    if !p.is_empty() {
        return Err("trailing footer bytes");
    }
    Ok(Rule {
        std,
        dst: Some((dst, d1, t1, d2, t2)),
    })
}

fn name<'a>(p: &mut &'a [u8]) -> Result<&'a [u8], &'static str> {
    if let Some(rest) = p.strip_prefix(b"<") {
        let end = rest
            .iter()
            .position(|&b| b == b'>')
            .ok_or("unterminated <name>")?;
        let n = &rest[..end];
        if n.len() < 3
            || !n
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b'+' || *b == b'-')
        {
            return Err("bad quoted name");
        }
        *p = &rest[end + 1..];
        Ok(n)
    } else {
        let end = p
            .iter()
            .position(|b| !b.is_ascii_alphabetic())
            .unwrap_or(p.len());
        if end < 3 {
            return Err("short name");
        }
        let n = &p[..end];
        *p = &p[end..];
        Ok(n)
    }
}

fn num(p: &mut &[u8], max: i32) -> Result<i32, &'static str> {
    let end = p
        .iter()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(p.len());
    if end == 0 || end > 3 {
        return Err("bad number");
    }
    let mut v = 0i32;
    for &b in &p[..end] {
        v = v * 10 + i32::from(b - b'0');
    }
    *p = &p[end..];
    if v > max {
        Err("number out of range")
    } else {
        Ok(v)
    }
}

/// `[+-]hh[:mm[:ss]]` in seconds, POSIX sign (positive is west of UTC).
fn offset(p: &mut &[u8], max_h: i32) -> Result<i32, &'static str> {
    let neg = match p.first() {
        Some(b'-') => true,
        Some(b'+') => false,
        _ => {
            return hms(p, max_h);
        }
    };
    *p = &p[1..];
    let v = hms(p, max_h)?;
    Ok(if neg { -v } else { v })
}

fn hms(p: &mut &[u8], max_h: i32) -> Result<i32, &'static str> {
    let mut v = num(p, max_h)? * 3600;
    if let Some(rest) = p.strip_prefix(b":") {
        *p = rest;
        v += num(p, 59)? * 60;
        if let Some(rest) = p.strip_prefix(b":") {
            *p = rest;
            v += num(p, 59)?;
        }
    }
    Ok(v)
}

fn date_time(p: &mut &[u8]) -> Result<(Date, i32), &'static str> {
    let d = if let Some(rest) = p.strip_prefix(b"J") {
        *p = rest;
        let n = num(p, 365)?;
        if n == 0 {
            return Err("J0");
        }
        Date::Julian1(n as u16)
    } else if let Some(rest) = p.strip_prefix(b"M") {
        *p = rest;
        let m = num(p, 12)?;
        *p = p.strip_prefix(b".").ok_or("bad M rule")?;
        let w = num(p, 5)?;
        *p = p.strip_prefix(b".").ok_or("bad M rule")?;
        let d = num(p, 6)?;
        if m == 0 || w == 0 {
            return Err("bad M rule");
        }
        Date::Month {
            m: m as u8,
            w: w as u8,
            d: d as u8,
        }
    } else {
        Date::Julian0(num(p, 365)? as u16)
    };
    // RFC 8536 3.3.1: the time may be negative and up to 167 hours.
    let t = if let Some(rest) = p.strip_prefix(b"/") {
        *p = rest;
        offset(p, 167)?
    } else {
        7200
    };
    Ok((d, t))
}

impl Rule {
    fn at(&self, t: i64) -> Type {
        let Some((dst, d1, t1, d2, t2)) = self.dst else {
            return self.std;
        };
        // The local standard-time year of `t`.
        let year = civil_from_days((t + i64::from(self.std.utoff)).div_euclid(86400)).0;
        let start = day_of(year, d1) * 86400 + i64::from(t1) - i64::from(self.std.utoff);
        let end = day_of(year, d2) * 86400 + i64::from(t2) - i64::from(dst.utoff);
        let in_dst = if start <= end {
            start <= t && t < end
        } else {
            !(end <= t && t < start)
        };
        if in_dst { dst } else { self.std }
    }
}

fn is_leap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

fn day_of(year: i64, d: Date) -> i64 {
    let jan1 = days_from_civil(year, 1, 1);
    match d {
        Date::Julian1(n) => {
            let n = i64::from(n) - 1;
            jan1 + n + i64::from(is_leap(year) && n >= 59)
        }
        Date::Julian0(n) => jan1 + i64::from(n),
        Date::Month { m, w, d } => {
            let first = days_from_civil(year, u32::from(m), 1);
            // 1970-01-01 was a Thursday (4).
            let wd_first = (first + 4).rem_euclid(7);
            let mut day = (i64::from(d) - wd_first).rem_euclid(7) + (i64::from(w) - 1) * 7;
            let dim = days_in_month(year, u32::from(m));
            while day >= dim {
                day -= 7;
            }
            first + day
        }
    }
}

pub fn days_in_month(y: i64, m: u32) -> i64 {
    match m {
        2 => 28 + i64::from(is_leap(y)),
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Howard Hinnant's `days_from_civil`.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Howard Hinnant's `civil_from_days`: (year, month 1-12, day 1-31).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
