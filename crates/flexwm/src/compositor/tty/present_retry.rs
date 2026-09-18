//! How many times a refused commit/page-flip may arm a timer-driven retry render.
//!
//! A refused flip is the one present skip no completion event can retry: an
//! in-flight skip is owed a `VBlank` (its flip is still out, and
//! `present_skipped` converts that into a re-render), but a refused flip
//! issued nothing, so at idle no event would ever re-trigger the frame and
//! the failed content would sit stale until unrelated damage arrived. The
//! render tail therefore re-arms the frame timer directly (see
//! `Tty::take_retry_render`) -- but only up to a bound: a persistently
//! refusing device must not pin the loop at 60Hz full redraws forever, which
//! would trade a rare stale frame for constant CPU burn on dead hardware.
//! The bound is small because the reachable case is transient (an EBUSY
//! against a stale flip a scanout invalidation discarded, resolved once its
//! vblank lands): three retries cover two still-racing ticks with one to
//! spare, and any issued flip resets the streak.

/// Consecutive refused flips that may each arm a retry render before the
/// compositor goes quiet until genuine damage arrives.
pub(super) const MAX_CONSECUTIVE_RETRIES: u32 = 3;

/// Counts consecutive commit/page-flip refusals. One struct rather than a
/// bare counter on `Tty` so the cap-and-reset arithmetic is unit-testable:
/// no test harness can construct a `Tty` (it needs a live DRM device).
pub(super) struct PresentRetries {
    consecutive: u32,
}

/// What a refused flip asks of the render tail.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Retry {
    /// Arm a retry render on the frame timer.
    Arm,
    /// The first refusal past the cap: the call site logs it once, then
    /// stays quiet until genuine damage arrives.
    GiveUp,
    /// Past the cap and already logged: nothing to do.
    Quiet,
}

impl PresentRetries {
    pub(super) fn new() -> Self {
        Self { consecutive: 0 }
    }

    /// Records a refused flip. Saturates rather than wrapping: past the cap
    /// every further refusal keeps answering `Quiet` with no overflow
    /// question at all.
    pub(super) fn failed(&mut self) -> Retry {
        self.consecutive = self.consecutive.saturating_add(1);
        if self.consecutive <= MAX_CONSECUTIVE_RETRIES {
            Retry::Arm
        } else if self.consecutive == MAX_CONSECUTIVE_RETRIES + 1 {
            Retry::GiveUp
        } else {
            Retry::Quiet
        }
    }

    /// Records an issued flip: the device takes commits again, so the next
    /// refusal starts a fresh streak.
    pub(super) fn succeeded(&mut self) {
        self.consecutive = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_three_refusals_arm_a_retry_then_one_gives_up_then_quiet() {
        // Fail-first pin for the bounded retry: transient EBUSY needs at
        // most a couple of ticks to clear, a wedged device must not retry
        // forever, and the give-up is exactly one log line per streak.
        // Neuter check: cap the constant at 0 and the `Arm` assertions fail.
        let mut retries = PresentRetries::new();
        assert_eq!(retries.failed(), Retry::Arm);
        assert_eq!(retries.failed(), Retry::Arm);
        assert_eq!(retries.failed(), Retry::Arm);
        assert_eq!(retries.failed(), Retry::GiveUp);
        assert_eq!(retries.failed(), Retry::Quiet);
        assert_eq!(retries.failed(), Retry::Quiet);
    }

    #[test]
    fn an_issued_flip_resets_the_streak() {
        let mut retries = PresentRetries::new();
        assert_eq!(retries.failed(), Retry::Arm);
        assert_eq!(retries.failed(), Retry::Arm);
        assert_eq!(retries.failed(), Retry::Arm);
        retries.succeeded();
        assert_eq!(retries.failed(), Retry::Arm);
        assert_eq!(retries.failed(), Retry::Arm);
        assert_eq!(retries.failed(), Retry::Arm);
        assert_eq!(retries.failed(), Retry::GiveUp);
    }

    #[test]
    fn a_long_refusal_streak_saturates_quietly() {
        let mut retries = PresentRetries::new();
        for _ in 0..100 {
            retries.failed();
        }
        assert_eq!(retries.failed(), Retry::Quiet);
        retries.succeeded();
        assert_eq!(retries.failed(), Retry::Arm);
    }
}
