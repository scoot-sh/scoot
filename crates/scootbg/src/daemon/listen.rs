//! Whether the loop polls the control socket's listener.
//!
//! Normally always (see `control`'s module docs for why). When accepting
//! fails in a way that retrying at once would only repeat (out of file
//! descriptors with none of its own to free, `ENOMEM`, `ENOBUFS`, ...),
//! the daemon used to exit: right for a daemon that only answers a socket,
//! wrong once it holds the wallpaper, which would vanish with it. Instead
//! the listener *rests*:
//!
//! - it leaves the poll set, so a connection it cannot accept does not
//!   keep the level-triggered `poll` returning (no spin);
//! - the poll gets a timeout of [`REST`], so the listener is re-armed and
//!   tried again (never deaf for longer than that);
//! - the wallpaper surfaces and the Wayland connection carry on meanwhile.
//!
//! A timeout exists only while resting: an armed listener polls with none,
//! so an idle daemon still makes no wakeups at all. A failure is reported
//! once, when it starts, and its end once, at the first accept that works,
//! so a condition that lasts does not print a line a second.

use std::time::{Duration, Instant};

/// How long the listener rests after a failed accept.
pub const REST: Duration = Duration::from_secs(1);

#[derive(Debug, Default)]
pub struct Listening {
    /// While resting: when to re-arm.
    until: Option<Instant>,
    /// A failure was reported and no accept has worked since.
    reported: bool,
}

impl Listening {
    /// Whether to poll the listener this round, and the poll's timeout.
    /// `now` is called only while resting, so the normal path does not
    /// read the clock.
    pub fn poll_plan(&mut self, now: impl FnOnce() -> Instant) -> (bool, Option<Duration>) {
        let Some(until) = self.until else {
            return (true, None);
        };
        let left = until.saturating_duration_since(now());
        if left.is_zero() {
            self.until = None;
            (true, None)
        } else {
            (false, Some(left))
        }
    }

    /// An accept round failed at `now`. Returns whether to report it (the
    /// first failure since accepting last worked).
    pub fn failed(&mut self, now: Instant) -> bool {
        // `checked_add` cannot fail: the monotonic clock counts from boot
        // and is centuries from overflowing. `unwrap_or` only keeps this
        // free of a panic path.
        self.until = Some(now.checked_add(REST).unwrap_or(now));
        !std::mem::replace(&mut self.reported, true)
    }

    /// An accept round worked. Returns whether that ends a reported
    /// failure.
    pub fn worked(&mut self) -> bool {
        std::mem::replace(&mut self.reported, false)
    }

    #[cfg(test)]
    pub fn is_resting(&self) -> bool {
        self.until.is_some()
    }
}
