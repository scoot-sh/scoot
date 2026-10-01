//! How long to wait before starting an `exec` module's command again.
//!
//! The first restart waits [`Restart::first`], each one after it twice the
//! last, up to [`Restart::max`]: a command that cannot start (a missing
//! program) or dies at once is tried less and less often, never in a spin.
//! A run that lasted [`Restart::stable`] or longer starts the sequence over,
//! so a script that dies once a day is back in a second, not a minute.

use std::time::Duration;

/// The restart policy, in the config's defaults unless a test says
/// otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Restart {
    pub first: Duration,
    pub max: Duration,
    pub stable: Duration,
}

impl Default for Restart {
    fn default() -> Self {
        Self {
            first: Duration::from_secs(1),
            max: Duration::from_secs(60),
            stable: Duration::from_secs(30),
        }
    }
}

#[derive(Debug)]
pub struct Backoff {
    policy: Restart,
    /// The wait the next restart takes.
    next: Duration,
}

impl Backoff {
    pub fn new(policy: Restart) -> Self {
        Self {
            policy,
            next: policy.first,
        }
    }

    /// A run of `lasted` just ended: how long to wait before the next one.
    pub fn after(&mut self, lasted: Duration) -> Duration {
        if lasted >= self.policy.stable {
            self.next = self.policy.first;
        }
        let wait = self.next;
        self.next = self.next.saturating_mul(2).min(self.policy.max);
        wait
    }
}
