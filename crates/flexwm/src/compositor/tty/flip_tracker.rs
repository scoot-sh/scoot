//! Which in-flight DRM flip is which: the sequence numbers behind the
//! session-lock vblank wait.
//!
//! `Tty::present` issues at most one flip at a time -- it skips while one is
//! still in flight, since flipping again would fail with EBUSY -- so "the
//! in-flight flip" is always at most one number, and matching a completion
//! against it is an equality check, not a search. The counter is `u64` and
//! monotonic while the process lives; wrapping is not a correctness question
//! (it would take hundreds of millions of years of one flip per vblank), so
//! this uses `wrapping_add` and moves on.
//!
//! Two facts share one field deliberately: `inflight.is_some()` *is* "a flip
//! is in flight", which is what `present` gates on and what the pause and
//! scanout-invalidation paths clear. A separate boolean beside it would be
//! the fact-in-two-places split this project treats as its own bug class.

/// The flip bookkeeping for one `--tty` presenter: the next sequence number
/// to hand out, and the in-flight flip's number, if one is out.
pub(super) struct FlipTracker {
    next: u64,
    inflight: Option<u64>,
}

impl FlipTracker {
    pub(super) fn new() -> Self {
        Self {
            next: 0,
            inflight: None,
        }
    }

    /// Whether a flip is still awaiting its completion -- what `present`
    /// gates on, and what a pause or a scanout invalidation clears.
    pub(super) fn is_busy(&self) -> bool {
        self.inflight.is_some()
    }

    /// Records a flip just issued to the kernel, returning its sequence
    /// number for whoever needs to recognize its completion later (the
    /// session-lock vblank wait). Call only when none is in flight --
    /// `present` guarantees that -- since a second number would silently
    /// orphan the first one's completion.
    pub(super) fn issued(&mut self) -> u64 {
        debug_assert!(
            self.inflight.is_none(),
            "a second flip issued while one is still in flight"
        );
        let seq = self.next;
        self.next = self.next.wrapping_add(1);
        self.inflight = Some(seq);
        seq
    }

    /// Records that whatever was in flight is done -- confirmed by a vblank,
    /// or untrackable after an error -- returning its number, or `None` when
    /// nothing was out (a stale vblank for a flip already discarded).
    pub(super) fn settled(&mut self) -> Option<u64> {
        self.inflight.take()
    }

    /// Forgets the in-flight flip without a completion: its vblank will
    /// never arrive, or arrives too late to mean anything. The pause path
    /// (the device is gone with the session) and the scanout invalidation
    /// (a modeset made every assumption about the CRTC void) both land here
    /// rather than leaving a number a later vblank could match against a
    /// lock wait recorded after it.
    pub(super) fn discard(&mut self) {
        self.inflight = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_issue_settle_roundtrip_hands_back_its_number() {
        let mut tracker = FlipTracker::new();
        assert!(!tracker.is_busy());
        let seq = tracker.issued();
        assert!(tracker.is_busy());
        assert_eq!(tracker.settled(), Some(seq));
        assert!(!tracker.is_busy());
    }

    #[test]
    fn settling_with_nothing_in_flight_is_none() {
        let mut tracker = FlipTracker::new();
        assert_eq!(tracker.settled(), None);
        // Twice: a stale second vblank for the same discarded flip.
        assert_eq!(tracker.settled(), None);
    }

    #[test]
    fn sequence_numbers_are_distinct_and_ordered() {
        let mut tracker = FlipTracker::new();
        let first = tracker.issued();
        assert_eq!(tracker.settled(), Some(first));
        let second = tracker.issued();
        assert_eq!(tracker.settled(), Some(second));
        assert!(
            second > first,
            "a wait recorded for {first} must not match {second}"
        );
    }

    #[test]
    fn discard_drops_the_number_a_late_vblank_would_match() {
        let mut tracker = FlipTracker::new();
        let seq = tracker.issued();
        tracker.discard();
        assert!(!tracker.is_busy());
        // The late vblank for `seq`: nothing is out, so it settles to `None`
        // rather than to the discarded number.
        assert_eq!(tracker.settled(), None);
        let _ = seq;
    }

    #[test]
    fn a_flip_issued_after_a_discard_gets_a_fresh_number() {
        let mut tracker = FlipTracker::new();
        let stale = tracker.issued();
        tracker.discard();
        let fresh = tracker.issued();
        assert_ne!(stale, fresh);
        assert_eq!(tracker.settled(), Some(fresh));
    }
}
