//! Expiry from first presentation: the spec counts `expire_timeout`
//! "since the display of the notification", so the clock starts when the
//! card is first presented — on the `wl_surface.frame` callback for the
//! commit that first contains it — never at arrival. A card that arrives
//! while locked or output-less therefore waits with its clock stopped.
//! Headless (no compositor) the daemon presents on arrival instead:
//! arrival is display, with nothing to hide behind. The tests' stub
//! presenter calls [`Clock::presented`] by hand, which is the seam the
//! real callback uses.
//!
//! Pure over an explicit `now`: every function takes it, so tests pin
//! the rules without sleeping.

use std::time::{Duration, Instant};

use crate::bus::hints::Urgency;
use crate::config::Config;

/// Resolves `expire_timeout` to a deadline in milliseconds, or `None`
/// for never: -1 is the per-urgency default (critical never expires
/// unless the client asked with a positive value), 0 is never, a
/// positive value is honored up to the config's clamp. Any other
/// negative reads as -1: the spec names only -1, and guessing a second
/// meaning for one would invent protocol.
pub fn resolve_timeout(expire_timeout: i32, urgency: Urgency, config: &Config) -> Option<u32> {
    match expire_timeout {
        0 => None,
        _ if expire_timeout < 0 => match urgency {
            Urgency::Low => Some(config.timeout_low_ms),
            Urgency::Normal => Some(config.timeout_normal_ms),
            Urgency::Critical => None,
        },
        _ => Some((expire_timeout as u32).min(config.timeout_max_ms)),
    }
}

/// One card's expiry clock: stopped until [`Clock::presented`], pausable
/// while idled, resumable after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clock {
    deadline: Option<Instant>,
    remaining_ms: Option<u32>,
    paused: bool,
    shown: bool,
}

impl Clock {
    /// A clock that has not started: the card is waiting to be shown.
    pub fn waiting() -> Self {
        Self {
            deadline: None,
            remaining_ms: None,
            paused: false,
            shown: false,
        }
    }

    /// Whether the card was ever presented.
    pub fn shown(&self) -> bool {
        self.shown
    }

    /// First presentation: the clock starts at `now + timeout_ms`, or
    /// stays stopped for never. Re-presenting a shown card is a no-op
    /// (replace restarts through [`Clock::restart`], not here).
    pub fn presented(&mut self, now: Instant, timeout_ms: Option<u32>) {
        if self.shown {
            return;
        }
        self.shown = true;
        self.paused = false;
        self.remaining_ms = None;
        self.deadline = timeout_ms.map(|ms| now + Duration::from_millis(u64::from(ms)));
    }

    /// A replace on a shown card: the expiry restarts from `now`.
    pub fn restart(&mut self, now: Instant, timeout_ms: Option<u32>) {
        if !self.shown {
            return;
        }
        self.paused = false;
        self.remaining_ms = None;
        self.deadline = timeout_ms.map(|ms| now + Duration::from_millis(u64::from(ms)));
    }

    /// Idle began: a running deadline parks as its remaining time.
    pub fn pause(&mut self, now: Instant) {
        if self.paused {
            return;
        }
        self.paused = true;
        if let Some(deadline) = self.deadline {
            let ms = deadline
                .saturating_duration_since(now)
                .as_millis()
                .min(u128::from(u32::MAX)) as u32;
            self.remaining_ms = Some(ms);
            self.deadline = None;
        }
    }

    /// Idle ended: the parked time runs again from `now`.
    pub fn resume(&mut self, now: Instant) {
        if !self.paused {
            return;
        }
        self.paused = false;
        if let Some(ms) = self.remaining_ms.take()
            && self.shown
        {
            self.deadline = Some(now + Duration::from_millis(u64::from(ms)));
        }
    }

    /// Whether the deadline passed: only a running, shown clock fires.
    pub fn due(&self, now: Instant) -> bool {
        !self.paused && self.shown && self.deadline.is_some_and(|at| now >= at)
    }

    /// The running deadline, for the timerfd's earliest arm.
    pub fn deadline(&self) -> Option<Instant> {
        if self.paused || !self.shown {
            None
        } else {
            self.deadline
        }
    }
}

#[cfg(test)]
mod tests;
