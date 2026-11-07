//! Where the sun is: the pure half of `scoot look auto --sun`.
//!
//! One function, [`altitude_deg`], gives the sun's height above the horizon
//! for a place and an instant (the low-precision solar position of NOAA's
//! solar calculator, after Meeus, *Astronomical Algorithms*; good to about
//! a minute for sunrise and sunset between the polar circles). Everything
//! else is derived from it: [`is_day`] asks whether that height clears the
//! sunrise/sunset altitude, and [`next_change`] scans forward for the
//! instant the answer flips. There is no separate sunrise formula, so there
//! is no polar special case: a place with no sunrise or sunset in the
//! scanned span has no change in it, and the previous side simply stays.
//!
//! Every instant is Unix seconds (UTC). No time zone and no daylight-saving
//! rule is involved, so a caller never has to name either. No I/O, no
//! clock reads, no allocation.

pub mod civil;

use std::f64::consts::PI;

const RAD: f64 = PI / 180.0;

/// The altitude of the Sun's centre at sunrise and sunset: the Sun's
/// 16' semi-diameter plus 34' of atmospheric refraction, the convention
/// almanacs (the US Naval Observatory's included) publish times for.
pub const HORIZON_DEG: f64 = -0.833;

/// Degrees are held as whole micro-degrees (0.11 m), so a location is
/// exactly comparable, prints back exactly as it was written, and fits in
/// two `i32`s.
const MICRO: f64 = 1_000_000.0;

/// How far apart [`next_change`] samples the altitude. A dip below the
/// horizon shorter than this (grazing the horizon near a polar day's end)
/// is not a change worth a swap.
const SCAN_STEP_SECS: i64 = 60;

/// A place on Earth: latitude north-positive in -90 to 90, longitude
/// east-positive in -180 to 180, to micro-degree precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    lat_micro: i32,
    lon_micro: i32,
}

impl Location {
    /// A location from degrees; `None` outside the ranges or not finite.
    /// The values round to the nearest micro-degree.
    pub fn new(lat: f64, lon: f64) -> Option<Location> {
        let micro = |degrees: f64, limit: f64| {
            (degrees.is_finite() && degrees.abs() <= limit)
                // In range by the check above, so the cast cannot saturate.
                .then(|| (degrees * MICRO).round() as i32)
        };
        Some(Location {
            lat_micro: micro(lat, 90.0)?,
            lon_micro: micro(lon, 180.0)?,
        })
    }

    /// Parses the CLI spelling of the two coordinates: [`parse_degrees`]
    /// each, then the range check.
    pub fn parse(lat: &str, lon: &str) -> Option<Location> {
        Location::new(parse_degrees(lat)?, parse_degrees(lon)?)
    }

    pub fn latitude(self) -> f64 {
        f64::from(self.lat_micro) / MICRO
    }

    pub fn longitude(self) -> f64 {
        f64::from(self.lon_micro) / MICRO
    }

    /// The latitude as [`Location::parse`] reads it back: the shortest
    /// decimal that is exact to a micro-degree (`51.5`, `-0.127758`, `0`).
    pub fn latitude_text(self) -> String {
        micro_text(self.lat_micro)
    }

    /// The longitude, as [`Location::latitude_text`].
    pub fn longitude_text(self) -> String {
        micro_text(self.lon_micro)
    }
}

/// `51500000` as `51.5`: sign, whole degrees, then the fraction with its
/// trailing zeros dropped. Zero is `0` whatever its sign.
fn micro_text(micro: i32) -> String {
    let sign = if micro < 0 { "-" } else { "" };
    let abs = micro.unsigned_abs();
    let (whole, frac) = (abs / 1_000_000, abs % 1_000_000);
    if frac == 0 {
        return format!("{sign}{whole}");
    }
    let frac = format!("{frac:06}");
    format!("{sign}{whole}.{}", frac.trim_end_matches('0'))
}

/// Decimal degrees in the one spelling the command line takes: an optional
/// `-`, digits, and optionally a `.` with digits after it. No `+`, no
/// exponent, no `nan` or `inf`, no spaces, no bare `.5` or `5.`, and at
/// most twenty digits so no input is long enough to matter. The range is
/// [`Location::new`]'s job.
pub fn parse_degrees(text: &str) -> Option<f64> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let (whole, frac) = match digits.split_once('.') {
        Some((whole, frac)) => (whole, Some(frac)),
        None => (digits, None),
    };
    let all_digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(whole) || frac.is_some_and(|frac| !all_digits(frac)) || digits.len() > 20 {
        return None;
    }
    text.parse().ok()
}

/// The Sun's altitude above the horizon in degrees (negative below it),
/// seen from `place` at `unix` seconds UTC. Refraction is not included:
/// compare against [`HORIZON_DEG`].
pub fn altitude_deg(place: Location, unix: i64) -> f64 {
    // Julian centuries since J2000.0.
    let jd = unix as f64 / 86_400.0 + 2_440_587.5;
    let t = (jd - 2_451_545.0) / 36_525.0;

    let l0 = (280.46646 + t * (36000.76983 + t * 0.0003032)).rem_euclid(360.0) * RAD;
    let m = (357.52911 + t * (35999.05029 - 0.0001537 * t)) * RAD;
    let e = 0.016708634 - t * (0.000042037 + 0.0000001267 * t);
    let centre = m.sin() * (1.914602 - t * (0.004817 + 0.000014 * t))
        + (2.0 * m).sin() * (0.019993 - 0.000101 * t)
        + (3.0 * m).sin() * 0.000289;
    let omega = (125.04 - 1934.136 * t) * RAD;
    let lambda = (l0 / RAD + centre - 0.00569 - 0.00478 * omega.sin()) * RAD;
    let eps = (23.0
        + (26.0 + (21.448 - t * (46.815 + t * (0.00059 - t * 0.001813))) / 60.0) / 60.0
        + 0.00256 * omega.cos())
        * RAD;
    let declination = (eps.sin() * lambda.sin()).asin();

    // Equation of time, minutes.
    let y = (eps / 2.0).tan().powi(2);
    let eq_time = 4.0 / RAD
        * (y * (2.0 * l0).sin() - 2.0 * e * m.sin() + 4.0 * e * y * m.sin() * (2.0 * l0).cos()
            - 0.5 * y * y * (4.0 * l0).sin()
            - 1.25 * e * e * (2.0 * m).sin());

    // True solar time in minutes, then the hour angle (0 at solar noon).
    let minutes = unix.rem_euclid(86_400) as f64 / 60.0;
    let solar_minutes = minutes + eq_time + 4.0 * place.longitude();
    let hour_angle = (solar_minutes / 4.0 - 180.0) * RAD;

    let lat = place.latitude() * RAD;
    let sin_alt = lat.sin() * declination.sin() + lat.cos() * declination.cos() * hour_angle.cos();
    sin_alt.clamp(-1.0, 1.0).asin() / RAD
}

/// Whether the Sun is up at `place` at `unix`: its centre above
/// [`HORIZON_DEG`]. Polar day is day all day and polar night is night all
/// day, with no special case.
pub fn is_day(place: Location, unix: i64) -> bool {
    altitude_deg(place, unix) > HORIZON_DEG
}

/// A flip of [`is_day`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    /// The first whole second at which [`is_day`] has the new answer.
    pub at: i64,
    /// What it answers from then on (`true`: a sunrise, `false`: a sunset).
    pub day: bool,
}

/// The first flip of [`is_day`] after `after`, within `within` seconds, or
/// `None` when there is none (a polar day or night, or a short span).
/// Scans a minute at a time and bisects the bracket to the second:
/// 2160 altitude evaluations for 36 hours, microseconds in all.
pub fn next_change(place: Location, after: i64, within: i64) -> Option<Change> {
    let end = after.checked_add(within)?;
    let start = is_day(place, after);
    let mut before = after;
    while before < end {
        let probe = before.saturating_add(SCAN_STEP_SECS).min(end);
        if is_day(place, probe) != start {
            // The answer at `lo` is the old one, at `hi` the new one.
            let (mut lo, mut hi) = (before, probe);
            while hi - lo > 1 {
                let mid = lo + (hi - lo) / 2;
                if is_day(place, mid) == start {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            return Some(Change {
                at: hi,
                day: !start,
            });
        }
        before = probe;
    }
    None
}

#[cfg(test)]
mod tests;
