//! Whether the loop polls the control socket's listener.
//!
//! Normally always (see `control`'s module docs for why). When accepting
//! fails in a way retrying would only repeat, the listener *rests*:
//!
//! - it keeps its slot in the poll set but asks for no events, so a
//!   connection it cannot accept does not keep the level-triggered `poll`
//!   returning (no spin).
//! - the poll gets a timeout of [`REST`], so the listener is re-armed and
//!   tried again once it lapses.
//!
//! A timeout exists only while resting: an armed listener polls with none,
//! so the control socket adds no idle wakeups.
//!
//! (As scootbg's `daemon/listen.rs`.)

use std::time::{Duration, Instant};

/// How long the listener rests after a failed accept.
pub const REST: Duration = Duration::from_secs(1);

/// The listener's state.
#[derive(Debug, Default)]
pub struct Listening {
    /// Resting until this time.
    until: Option<Instant>,
}

impl Listening {
    /// Whether to poll the listener this round, and the poll's timeout.
    pub fn poll_plan(&mut self, now: impl FnOnce() -> Instant) -> (bool, Option<Duration>) {
        match self.until {
            None => (true, None),
            Some(until) => {
                let now = now();
                if now >= until {
                    self.until = None;
                    (true, None)
                } else {
                    (false, Some(until - now))
                }
            }
        }
    }

    /// The accept failed: rest the listener.
    pub fn rest(&mut self, now: Instant) {
        self.until = Some(now + REST);
    }
}
