//! Throwaway spike: a minute clock on an absolute CLOCK_REALTIME timerfd
//! with TFD_TIMER_CANCEL_ON_SET, plus interchangeable zone backends so the
//! hand-rolled TZif reader can be weighed against the crates.
//!
//! Usage:
//!   sb-clock-spike run   [ZONEFILE]      the clock loop, one line per wake
//!   sb-clock-spike bench ZONEFILE N      N conversions + formats, ns/op
//!   sb-clock-spike dump  ZONEFILE        (hand only) unix times on stdin ->
//!                                        "t utoff isdst abbr" on stdout
//! ZONEFILE defaults to /etc/localtime. The chrono and libc backends ignore
//! it and read TZ / /etc/localtime themselves.

use std::fmt::Write as _;
use std::io::Write as _;

use rustix::event::{PollFd, PollFlags, poll};
use rustix::fs::{AtFlags, CWD, StatxFlags, statx};
use rustix::io::Errno;
use rustix::time::{
    ClockId, Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, clock_gettime,
    timerfd_create, timerfd_settime,
};

#[cfg(not(any(
    feature = "tzrs",
    feature = "jiff",
    feature = "chrono",
    feature = "libc",
    feature = "utc"
)))]
mod tzif;

/// A fixed buffer; formatting never allocates.
struct Out {
    buf: [u8; 64],
    len: usize,
}
impl std::fmt::Write for Out {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let end = self.len.checked_add(s.len()).ok_or(std::fmt::Error)?;
        self.buf
            .get_mut(self.len..end)
            .ok_or(std::fmt::Error)?
            .copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}
impl Out {
    fn new() -> Out {
        Out {
            buf: [0; 64],
            len: 0,
        }
    }
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.buf[..self.len]).unwrap_or("?")
    }
}

const WDAY: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MON: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

#[allow(clippy::too_many_arguments)]
fn put(o: &mut Out, wday: usize, d: u32, m: u32, y: i64, hh: u32, mm: u32, abbr: &str) {
    let _ = write!(
        o,
        "{} {:02} {} {} {:02}:{:02} {}",
        WDAY[wday % 7],
        d,
        MON[(m as usize).saturating_sub(1) % 12],
        y,
        hh,
        mm,
        abbr
    );
}

// ---------------------------------------------------------------- backends

#[cfg(not(any(
    feature = "tzrs",
    feature = "jiff",
    feature = "chrono",
    feature = "libc",
    feature = "utc"
)))]
mod zone {
    use super::*;
    pub struct Zone(pub tzif::Tz);
    pub fn load(path: &str) -> Zone {
        let tz = std::fs::read(path)
            .ok()
            .and_then(|b| tzif::Tz::parse(&b).ok())
            .unwrap_or_else(tzif::Tz::utc);
        Zone(tz)
    }
    impl Zone {
        pub fn utoff(&self, t: i64) -> i64 {
            i64::from(self.0.at(t).0.utoff)
        }
        pub fn fmt(&self, t: i64, o: &mut Out) {
            let (l, a) = self.0.at(t);
            let lt = t + i64::from(l.utoff);
            let days = lt.div_euclid(86400);
            let sod = lt.rem_euclid(86400) as u32;
            let (y, m, d) = tzif::civil_from_days(days);
            let wday = (days + 4).rem_euclid(7) as usize;
            let n = a.iter().position(|&b| b == 0).unwrap_or(a.len());
            put(
                o,
                wday,
                d,
                m,
                y,
                sod / 3600,
                sod / 60 % 60,
                std::str::from_utf8(&a[..n]).unwrap_or("?"),
            );
        }
    }
}

#[cfg(feature = "tzrs")]
mod zone {
    use super::*;
    pub struct Zone(tz::TimeZone);
    pub fn load(path: &str) -> Zone {
        let tz = std::fs::read(path)
            .ok()
            .and_then(|b| tz::TimeZone::from_tz_data(&b).ok())
            .unwrap_or_else(tz::TimeZone::utc);
        Zone(tz)
    }
    impl Zone {
        pub fn utoff(&self, t: i64) -> i64 {
            self.0
                .find_local_time_type(t)
                .map_or(0, |l| i64::from(l.ut_offset()))
        }
        pub fn fmt(&self, t: i64, o: &mut Out) {
            if let Ok(dt) = tz::DateTime::from_timespec(t, 0, self.0.as_ref()) {
                let abbr = dt.local_time_type().time_zone_designation();
                put(
                    o,
                    dt.week_day() as usize,
                    u32::from(dt.month_day()),
                    u32::from(dt.month()),
                    i64::from(dt.year()),
                    u32::from(dt.hour()),
                    u32::from(dt.minute()),
                    abbr,
                );
            }
        }
    }
}

#[cfg(feature = "jiff")]
mod zone {
    use super::*;
    pub struct Zone(jiff::tz::TimeZone);
    pub fn load(path: &str) -> Zone {
        let tz = std::fs::read(path)
            .ok()
            .and_then(|b| jiff::tz::TimeZone::tzif("local", &b).ok())
            .unwrap_or(jiff::tz::TimeZone::UTC);
        Zone(tz)
    }
    impl Zone {
        pub fn utoff(&self, t: i64) -> i64 {
            jiff::Timestamp::from_second(t)
                .map_or(0, |ts| i64::from(self.0.to_offset(ts).seconds()))
        }
        pub fn fmt(&self, t: i64, o: &mut Out) {
            if let Ok(ts) = jiff::Timestamp::from_second(t) {
                let info = self.0.to_offset_info(ts);
                let dt = self.0.to_datetime(ts);
                put(
                    o,
                    dt.weekday().to_sunday_zero_offset() as usize,
                    dt.day() as u32,
                    dt.month() as u32,
                    i64::from(dt.year()),
                    dt.hour() as u32,
                    dt.minute() as u32,
                    info.abbreviation(),
                );
            }
        }
    }
}

#[cfg(feature = "chrono")]
mod zone {
    use super::*;
    use chrono::{Datelike, Offset, TimeZone, Timelike};
    pub struct Zone;
    pub fn load(_path: &str) -> Zone {
        Zone
    }
    impl Zone {
        pub fn utoff(&self, t: i64) -> i64 {
            chrono::Local
                .timestamp_opt(t, 0)
                .single()
                .map_or(0, |d| i64::from(d.offset().fix().local_minus_utc()))
        }
        pub fn fmt(&self, t: i64, o: &mut Out) {
            if let Some(dt) = chrono::Local.timestamp_opt(t, 0).single() {
                put(
                    o,
                    dt.weekday().num_days_from_sunday() as usize,
                    dt.day(),
                    dt.month(),
                    i64::from(dt.year()),
                    dt.hour(),
                    dt.minute(),
                    "",
                );
            }
        }
    }
}

#[cfg(feature = "libc")]
mod zone {
    use super::*;
    pub struct Zone;
    pub fn load(_path: &str) -> Zone {
        Zone
    }
    fn tm(t: i64) -> libc::tm {
        // SAFETY: an all-zero tm is valid; localtime_r writes it.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        let tt: libc::time_t = t;
        // SAFETY: both pointers are to live locals.
        unsafe { libc::localtime_r(&tt, &mut tm) };
        tm
    }
    impl Zone {
        pub fn utoff(&self, t: i64) -> i64 {
            tm(t).tm_gmtoff
        }
        pub fn fmt(&self, t: i64, o: &mut Out) {
            let tm = tm(t);
            let abbr = if tm.tm_zone.is_null() {
                ""
            } else {
                // SAFETY: glibc's tm_zone points at a static NUL-terminated string.
                unsafe { std::ffi::CStr::from_ptr(tm.tm_zone) }
                    .to_str()
                    .unwrap_or("?")
            };
            put(
                o,
                tm.tm_wday as usize,
                tm.tm_mday as u32,
                (tm.tm_mon + 1) as u32,
                i64::from(tm.tm_year) + 1900,
                tm.tm_hour as u32,
                tm.tm_min as u32,
                abbr,
            );
        }
    }
}

/// UTC only: the base the zone backends' sizes are measured over.
#[cfg(feature = "utc")]
mod zone {
    use super::*;
    pub struct Zone;
    pub fn load(_path: &str) -> Zone {
        Zone
    }
    fn civil(z: i64) -> (i64, u32, u32) {
        let z = z + 719468;
        let era = z.div_euclid(146097);
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        (yoe + era * 400 + i64::from(m <= 2), m, d)
    }
    impl Zone {
        pub fn utoff(&self, _t: i64) -> i64 {
            0
        }
        pub fn fmt(&self, t: i64, o: &mut Out) {
            let days = t.div_euclid(86400);
            let sod = t.rem_euclid(86400) as u32;
            let (y, m, d) = civil(days);
            put(
                o,
                (days + 4).rem_euclid(7) as usize,
                d,
                m,
                y,
                sod / 3600,
                sod / 60 % 60,
                "UTC",
            );
        }
    }
}

// ---------------------------------------------------------------- the loop

fn now(id: ClockId) -> Timespec {
    clock_gettime(id)
}

fn secs(ts: Timespec) -> f64 {
    ts.tv_sec as f64 + ts.tv_nsec as f64 / 1e9
}

/// The next local minute boundary strictly after `t`.
fn next_boundary(t: i64, utoff: i64) -> i64 {
    ((t + utoff).div_euclid(60) + 1) * 60 - utoff
}

fn arm(tfd: &rustix::fd::OwnedFd, z: &zone::Zone) -> i64 {
    loop {
        let t = now(ClockId::Realtime).tv_sec;
        let next = next_boundary(t, z.utoff(t));
        let spec = Itimerspec {
            it_interval: Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: Timespec {
                tv_sec: next,
                tv_nsec: 0,
            },
        };
        timerfd_settime(
            tfd,
            TimerfdTimerFlags::ABSTIME | TimerfdTimerFlags::CANCEL_ON_SET,
            &spec,
        )
        .expect("timerfd_settime");
        // A step between reading the clock and arming is not caught by
        // CANCEL_ON_SET (it only reports steps after the arm), so check the
        // deadline is still the next boundary.
        let after = now(ClockId::Realtime).tv_sec;
        if after < next && next - after <= 60 {
            return next;
        }
    }
}

fn stat_key(path: &str) -> Option<(u64, u64, i64, i64, u64)> {
    let s = statx(
        CWD,
        path,
        AtFlags::empty(),
        StatxFlags::BASIC_STATS | StatxFlags::MTIME,
    )
    .ok()?;
    Some((
        u64::from(s.stx_dev_major) << 32 | u64::from(s.stx_dev_minor),
        s.stx_ino,
        s.stx_mtime.tv_sec,
        i64::from(s.stx_mtime.tv_nsec),
        s.stx_size,
    ))
}

fn run(path: &str) {
    let mut z = zone::load(path);
    let mut key = stat_key(path);
    let tfd = timerfd_create(
        TimerfdClockId::Realtime,
        TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK,
    )
    .expect("timerfd_create");
    let mut out = std::io::stdout().lock();
    let mut o = Out::new();
    z.fmt(now(ClockId::Realtime).tv_sec, &mut o);
    let _ = writeln!(
        out,
        "start mono={:.6} show=\"{}\"",
        secs(now(ClockId::Monotonic)),
        o.as_str()
    );
    let _ = out.flush();
    let mut deadline = arm(&tfd, &z);
    let (mut ticks, mut cancels) = (0u64, 0u64);
    loop {
        let mut fds = [PollFd::new(&tfd, PollFlags::IN)];
        match poll(&mut fds, None) {
            Ok(_) | Err(Errno::INTR) => {}
            Err(e) => panic!("poll: {e}"),
        }
        let mut b = [0u8; 8];
        let reason = match rustix::io::read(&tfd, &mut b) {
            Ok(8) => {
                ticks += 1;
                "tick"
            }
            Ok(_) => "short-read",
            Err(Errno::CANCELED) => {
                cancels += 1;
                "clock-set"
            }
            Err(Errno::AGAIN) => "spurious",
            Err(e) => panic!("read: {e}"),
        };
        let rt = now(ClockId::Realtime);
        // The zone file is re-checked on every wake: a stat, no extra fd, no
        // extra wakeup; a change shows within the minute.
        let k = stat_key(path);
        let reloaded = k != key;
        if reloaded {
            z = zone::load(path);
            key = k;
        }
        let mut o = Out::new();
        z.fmt(rt.tv_sec, &mut o);
        let late_us = (secs(rt) - deadline as f64) * 1e6;
        let _ = writeln!(
            out,
            "{reason} mono={:.6} real={}.{:09} late_us={:.0} ticks={ticks} cancels={cancels} reloaded={reloaded} show=\"{}\"",
            secs(now(ClockId::Monotonic)),
            rt.tv_sec,
            rt.tv_nsec,
            late_us,
            o.as_str()
        );
        let _ = out.flush();
        deadline = arm(&tfd, &z);
    }
}

fn bench(path: &str, n: u64) {
    let t = std::time::Instant::now();
    let z = zone::load(path);
    let load_ns = t.elapsed().as_nanos();
    let base = 1_790_000_000i64; // 2026-09-21
    let mut sum = 0u64;
    let t = std::time::Instant::now();
    for i in 0..n {
        let mut o = Out::new();
        z.fmt(base + (i as i64).wrapping_mul(997) % 31_622_400, &mut o);
        sum = sum
            .wrapping_add(o.len as u64)
            .wrapping_add(u64::from(o.buf[18]));
    }
    let el = t.elapsed().as_nanos();
    let mut o = Out::new();
    z.fmt(base, &mut o);
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let rss = status
        .lines()
        .find(|l| l.starts_with("VmRSS:"))
        .unwrap_or("");
    let anon = status
        .lines()
        .find(|l| l.starts_with("RssAnon:"))
        .unwrap_or("");
    println!(
        "load_ns={load_ns} ns_per_fmt={:.1} sample=\"{}\" sum={sum} {} {}",
        el as f64 / n as f64,
        o.as_str(),
        rss.split_whitespace().collect::<Vec<_>>().join(" "),
        anon.split_whitespace().collect::<Vec<_>>().join(" ")
    );
}

#[cfg(not(any(
    feature = "tzrs",
    feature = "jiff",
    feature = "chrono",
    feature = "libc",
    feature = "utc",
    feature = "bare"
)))]
fn dump(path: &str) {
    let tz = tzif::Tz::parse(&std::fs::read(path).expect("read zone")).expect("parse zone");
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for line in std::io::stdin().lines() {
        let line = line.expect("stdin");
        let t: i64 = line.trim().parse().expect("unix time");
        let (l, a) = tz.at(t);
        let n = a.iter().position(|&b| b == 0).unwrap_or(a.len());
        let _ = writeln!(
            out,
            "{t} {} {} {}",
            l.utoff,
            u8::from(l.isdst),
            std::str::from_utf8(&a[..n]).unwrap_or("?")
        );
    }
}

/// Truncate and bit-flip a zone file many ways; parse and query each. The
/// release profile aborts on panic, so finishing is the pass.
#[cfg(not(any(
    feature = "tzrs",
    feature = "jiff",
    feature = "chrono",
    feature = "libc",
    feature = "utc",
    feature = "bare"
)))]
fn mutate(path: &str, rounds: u64) {
    let orig = std::fs::read(path).expect("read zone");
    let (mut ok, mut err) = (0u64, 0u64);
    let mut probe = |b: &[u8]| match tzif::Tz::parse(b) {
        Ok(tz) => {
            for t in [
                i64::from(i32::MIN),
                -1,
                0,
                1_790_000_000,
                2_200_000_000,
                1 << 40,
            ] {
                std::hint::black_box(tz.at(t));
            }
            ok += 1;
        }
        Err(_) => err += 1,
    };
    for n in 0..orig.len() {
        probe(&orig[..n]);
    }
    let mut x = 0x9e3779b97f4a7c15u64;
    let mut buf = orig.clone();
    for _ in 0..rounds {
        buf.copy_from_slice(&orig);
        for _ in 0..1 + (x % 4) {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let i = (x as usize) % buf.len();
            buf[i] = (x >> 32) as u8;
        }
        probe(&buf);
    }
    println!("mutate {path}: parsed={ok} rejected={err}");
}

/// The same `dump` over tz-rs, so both readers face the same oracle.
#[cfg(all(feature = "tzrs", not(feature = "bare")))]
fn dump(path: &str) {
    let tz =
        tz::TimeZone::from_tz_data(&std::fs::read(path).expect("read zone")).expect("parse zone");
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for line in std::io::stdin().lines() {
        let line = line.expect("stdin");
        let t: i64 = line.trim().parse().expect("unix time");
        match tz.find_local_time_type(t) {
            Ok(l) => {
                let _ = writeln!(
                    out,
                    "{t} {} {} {}",
                    l.ut_offset(),
                    u8::from(l.is_dst()),
                    l.time_zone_designation()
                );
            }
            Err(_) => {
                let _ = writeln!(out, "{t} err 0 err");
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map_or("run", String::as_str);
    let path = args.get(2).map_or("/etc/localtime", String::as_str);
    match mode {
        "run" => run(path),
        "bench" => bench(
            path,
            args.get(3).map_or(1_000_000, |s| s.parse().expect("N")),
        ),
        #[cfg(not(any(
            feature = "tzrs",
            feature = "jiff",
            feature = "chrono",
            feature = "libc",
            feature = "utc",
            feature = "bare"
        )))]
        "dump" => dump(path),
        #[cfg(all(feature = "tzrs", not(feature = "bare")))]
        "dump" => dump(path),
        #[cfg(not(any(
            feature = "tzrs",
            feature = "jiff",
            feature = "chrono",
            feature = "libc",
            feature = "utc",
            feature = "bare"
        )))]
        "mutate" => mutate(path, args.get(3).map_or(100_000, |s| s.parse().expect("N"))),
        _ => eprintln!("usage: sb-clock-spike run|bench|dump [ZONEFILE] [N]"),
    }
}
