//! The easing curves the compositor's window slides and the shell apps'
//! card motion share, so a notification and a window move the same way.
//!
//! Pure arithmetic over `std::time` values: no clock is read here (callers
//! pass `now`), nothing allocates, and no curve needs a transcendental
//! function at run time: ease-out cubic is closed form, the spring is a
//! baked table. Progress is clamped to `0..=1` before it reaches any curve
//! and every curve lands exactly on `1.0`, so a late frame still snaps and
//! the caller still goes idle.

use std::time::{Duration, Instant};

/// Default slide length: short enough to feel instant, long enough to see.
pub const DEFAULT_DURATION_MS: u32 = 150;
/// Clamp range for `duration_ms`. Zero means "snap": a slide that finishes on
/// its first frame.
pub const MIN_DURATION_MS: u32 = 0;
pub const MAX_DURATION_MS: u32 = 500;

/// Which curve a slide follows from its start to its target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Easing {
    /// `1-(1-p)^3`. The default: fast off the mark, gentle landing.
    #[default]
    EaseOut,
    /// Straight-line interpolation. For testing and for anyone who wants it.
    Linear,
    /// An underdamped spring (damping ratio 0.6): overshoots about 9% past
    /// the target mid-flight, then settles back to an exact landing. Same
    /// duration bound as the other curves. For users who prefer niri-style
    /// motion.
    Spring,
}

impl Easing {
    /// Parses the `[animation] easing` spelling. `None` (absent key) is the
    /// default; `Err` carries the unknown name for the caller to warn about.
    pub fn parse(name: Option<&str>) -> Result<Self, String> {
        match name {
            None => Ok(Self::default()),
            Some(raw) => match raw.trim() {
                "ease-out" => Ok(Self::EaseOut),
                "linear" => Ok(Self::Linear),
                "spring" => Ok(Self::Spring),
                _ => Err(raw.to_owned()),
            },
        }
    }

    /// The config-file spelling, for reload reports.
    pub fn name(self) -> &'static str {
        match self {
            Self::EaseOut => "ease-out",
            Self::Linear => "linear",
            Self::Spring => "spring",
        }
    }
}

/// Fraction of an animation done at `now`, clamped `0..=1`. A zero duration
/// finishes on its first frame rather than dividing by zero.
#[inline]
pub fn progress(now: Instant, start: Instant, duration: Duration) -> f32 {
    if duration.is_zero() {
        return 1.0;
    }
    let elapsed = now.saturating_duration_since(start).as_secs_f32();
    (elapsed / duration.as_secs_f32()).clamp(0.0, 1.0)
}

/// The easing curve: progress in, eased progress out. Pure arithmetic, no
/// transcendental, no table — except the spring, which lerps a small baked
/// table (two loads and a multiply, still no transcendental).
#[inline]
pub fn easing_factor(progress: f32, easing: Easing) -> f32 {
    let clamped = progress.clamp(0.0, 1.0);
    match easing {
        Easing::Linear => clamped,
        Easing::EaseOut => {
            let inv = 1.0 - clamped;
            1.0 - inv * inv * inv
        }
        Easing::Spring => spring_factor(clamped),
    }
}

/// The spring curve, sampled from a 65-entry table over `p` in `0..=1`.
///
/// The entries bake the step response of an underdamped spring (damping
/// ratio 0.6, normalized frequency 7.0): fast off the mark, about 9%
/// overshoot past the target mid-flight, settled back by the deadline.
/// Endpoints are pinned (`[0] == 0.0`, `[64] == 1.0`) so the slide lands
/// exactly and prunes on time. Baked once at compile time so the frame
/// path pays no `exp`/`sin` per window per frame: just a table lookup and
/// a lerp. Clamped duration still bounds the settle, and a frame past its
/// deadline still snaps (`progress` reaches `1.0` first).
const SPRING_TABLE: [f32; 65] = [
    0.000000, 0.005722, 0.021877, 0.046999, //
    0.079705, 0.118695, 0.162760, 0.210781, //
    0.261736, 0.314691, 0.368808, 0.423338, //
    0.477619, 0.531074, 0.583206, 0.633594, //
    0.681888, 0.727804, 0.771121, 0.811673, //
    0.849347, 0.884074, 0.915828, 0.944619, //
    0.970489, 0.993507, 1.013767, 1.03138, //
    1.046476, 1.059194, 1.069685, 1.078105, //
    1.084615, 1.089376, 1.092551, 1.094299, //
    1.094775, 1.094131, 1.092513, 1.090057, //
    1.086895, 1.083148, 1.078931, 1.074349, //
    1.069498, 1.064465, 1.059329, 1.05416, //
    1.04902, 1.043964, 1.039037, 1.034279, //
    1.029722, 1.025393, 1.021312, 1.017495, //
    1.01395, 1.010686, 1.007703, 1.005001, //
    1.002576, 1.00042, 0.998524, 0.996878, //
    1.000000,
];

/// The spring easing curve: progress in (already clamped `0..=1`), eased
/// progress out. Overshoots past `1.0` mid-flight by design; lands exactly.
fn spring_factor(progress: f32) -> f32 {
    const LAST: f32 = (SPRING_TABLE.len() - 1) as f32;
    let scaled = progress * LAST;
    let low = scaled.floor() as usize;
    // `progress <= 1.0`, so `low <= 64`; the `min` is belt and braces.
    let low = low.min(SPRING_TABLE.len() - 1);
    let high = (low + 1).min(SPRING_TABLE.len() - 1);
    let t = scaled - low as f32;
    SPRING_TABLE[low] + (SPRING_TABLE[high] - SPRING_TABLE[low]) * t
}

/// The spring with its overshoot scaled by `bounce` (`0.0..=1.0`, clamped):
/// a straight blend of ease-out cubic and [`Easing::Spring`].
///
/// `0.0` is the ease-out curve (no overshoot), `1.0` is exactly the
/// compositor's spring (about 9% past the target mid-flight), and values
/// between scale the overshoot. Both curves are `0` at `0` and `1` at `1`,
/// so every blend lands exactly and the animation still ends on time.
#[inline]
pub fn bounce_factor(progress: f32, bounce: f32) -> f32 {
    let bounce = if bounce.is_nan() {
        0.0
    } else {
        bounce.clamp(0.0, 1.0)
    };
    let eased = easing_factor(progress, Easing::EaseOut);
    eased + bounce * (easing_factor(progress, Easing::Spring) - eased)
}

#[cfg(test)]
mod tests;
