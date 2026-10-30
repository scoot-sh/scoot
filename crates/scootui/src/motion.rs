//! Motion: a timeline helper the shell apps drive from frame callbacks.
//!
//! Pure arithmetic over `Instant` values the caller samples: this module
//! reads no clock, owns no timer and allocates nothing. An app asks "where
//! is it now?" on each `wl_surface.frame` callback while something moves,
//! and stops asking (and stops requesting callbacks) once every timeline
//! has settled, so an idle shell costs nothing.
//!
//! The curves are the compositor's own (`scoot_rt::easing`: ease-out cubic
//! and the baked spring table), so a notification card and a window move
//! the same way. A [`Timing`] picks one and clamps its inputs; a [`Settle`]
//! is an offset that decays from where it was to zero along it.
//!
//! A frame that arrives late jumps its progress forward (frames are
//! skipped, never queued) and a frame past the deadline snaps to the
//! target, because every curve lands exactly on `1.0`.

use std::time::{Duration, Instant};

use scoot_rt::easing::{Easing, bounce_factor, easing_factor, progress};

/// The longest a look may ask a motion to run, in milliseconds: past this
/// a notification stack is still moving when the next one arrives.
pub const MAX_DURATION_MS: u32 = 1000;
/// The duration when a look asks for motion and names none.
pub const DEFAULT_DURATION_MS: u32 = 250;
/// The spring's overshoot when a look names none: the compositor's own
/// spring, exactly.
pub const DEFAULT_BOUNCE: f32 = 1.0;

/// Which curve a motion follows (`motion = ...`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Curve {
    /// No motion: everything snaps. The default, and free.
    #[default]
    None,
    /// Ease-out cubic: fast off the mark, a gentle landing, no overshoot.
    EaseOut,
    /// The compositor's spring, its overshoot scaled by the bounce.
    Spring,
}

impl Curve {
    /// The curve a `motion` value names, if it names one.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "none" => Some(Self::None),
            "ease-out" => Some(Self::EaseOut),
            "spring" => Some(Self::Spring),
            _ => None,
        }
    }
}

/// A curve with its length and bounce, every input clamped: the one thing
/// a look's `motion`, `motion-duration-ms` and `motion-bounce` become.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timing {
    curve: Curve,
    duration: Duration,
    bounce: f32,
}

impl Default for Timing {
    /// No motion.
    fn default() -> Self {
        Self::NONE
    }
}

impl Timing {
    /// No motion: [`Timing::active`] is false and every query is settled.
    pub const NONE: Self = Self {
        curve: Curve::None,
        duration: Duration::ZERO,
        bounce: 0.0,
    };

    /// `curve` over `duration_ms` (at most [`MAX_DURATION_MS`]) with a
    /// spring `bounce` in `0.0..=1.0` (NaN reads as `0.0`). A duration of
    /// zero is no motion at all: it snaps, so it is [`Timing::NONE`].
    pub fn new(curve: Curve, duration_ms: u32, bounce: f32) -> Self {
        let duration = Duration::from_millis(u64::from(duration_ms.min(MAX_DURATION_MS)));
        if curve == Curve::None || duration.is_zero() {
            return Self::NONE;
        }
        let bounce = if bounce.is_nan() {
            0.0
        } else {
            bounce.clamp(0.0, 1.0)
        };
        Self {
            curve,
            duration,
            bounce,
        }
    }

    /// Whether this moves anything. False means every position snaps.
    pub fn active(&self) -> bool {
        self.curve != Curve::None
    }

    /// The curve.
    pub fn curve(&self) -> Curve {
        self.curve
    }

    /// The length of one motion.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// The same timing as an ease-out, for motion that must not overshoot
    /// (a card leaving): a spring becomes the ease-out of the same length.
    pub fn without_overshoot(&self) -> Self {
        match self.curve {
            Curve::Spring => Self {
                curve: Curve::EaseOut,
                ..*self
            },
            _ => *self,
        }
    }

    /// How far along the curve `start` is at `now`: `0.0` at the start,
    /// `1.0` from the deadline on, past `1.0` mid-flight only for a spring
    /// with a bounce. [`Timing::NONE`] is always `1.0`.
    pub fn eased(&self, start: Instant, now: Instant) -> f32 {
        match self.curve {
            Curve::None => 1.0,
            Curve::EaseOut => easing_factor(progress(now, start, self.duration), Easing::EaseOut),
            Curve::Spring => bounce_factor(progress(now, start, self.duration), self.bounce),
        }
    }

    /// Whether a motion begun at `start` has landed at `now`.
    pub fn settled(&self, start: Instant, now: Instant) -> bool {
        !self.active() || progress(now, start, self.duration) >= 1.0
    }
}

/// An offset that settles to zero: it is `from` at `start` and decays to
/// exactly zero along the timing's curve. A card's distance from where its
/// layout wants it is one: set it to the gap when the layout moves, and the
/// card is drawn at `target + value(now)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settle {
    from: f32,
    start: Instant,
}

impl Settle {
    /// An offset of `from` that begins settling at `start`.
    pub fn new(from: f32, start: Instant) -> Self {
        Self {
            from: if from.is_finite() { from } else { 0.0 },
            start,
        }
    }

    /// The offset it began from.
    pub fn from(&self) -> f32 {
        self.from
    }

    /// The offset at `now`. A spring overshoots: the sign flips briefly
    /// before it lands on zero.
    pub fn value(&self, timing: &Timing, now: Instant) -> f32 {
        if timing.settled(self.start, now) {
            return 0.0;
        }
        self.from * (1.0 - timing.eased(self.start, now))
    }

    /// Whether the offset has reached zero for good at `now`.
    pub fn settled(&self, timing: &Timing, now: Instant) -> bool {
        self.from == 0.0 || timing.settled(self.start, now)
    }
}

#[cfg(test)]
mod tests;
