//! Completions still owed to heads a hotplug tore down, and how long to wait
//! for them.
//!
//! A dumb-tier head removed with a flip still in flight is owed one `VBlank`
//! for that flip (see `Tty::stale_vblanks`): the next event on its CRTC is
//! the dead head's, not whatever head now drives that CRTC, so it must be
//! eaten rather than settled. In normal operation the owed event is already
//! queued on the DRM fd by the time the new head exists -- dropping the head
//! drops its surface, whose `Drop` does a blocking `ALLOW_MODESET` commit
//! that the kernel holds until the earlier flip completes -- so the next
//! event read for that CRTC is always the owed one, arriving within a frame
//! or two of the push.
//!
//! The residual is a driver that never delivers it (a kernel commit stall
//! timeout, or a driver fault): the entry would sit until the CRTC is reused
//! and then eat the new head's first real vblank, freezing that screen until
//! a VT switch or a DRM event error clears the list. So every entry carries
//! the push time, and a vblank first drops entries older than
//! [`STALE_VBLANK_MAX_AGE`] -- about one second, ~60 frames of grace against
//! a genuine event that is already queued at push time, while still far short
//! of any timescale a reused CRTC should wait out. The GPU tier needs no
//! equivalent: `DrmCompositor`'s queue is private, and its residual (a stale
//! completion settling a new frame at most one vblank early) is documented in
//! `scanout.rs`.
//!
//! Timestamps are explicit `Instant` parameters, the same idiom as
//! `SessionLock::await_vblank`'s `now`: no clock is read here, so the
//! age-out is unit-testable without a DRM device (no test harness can
//! construct a `Tty`).

use std::time::{Duration, Instant};

use smithay::reexports::drm::control::crtc;

/// How long a stale-vblank entry waits for its owed event before a vblank
/// stops believing it is still coming. See the module doc for why one
/// second is both ample (the genuine event is already queued at push time)
/// and bounded (a reused CRTC must not wait out a dead driver).
const STALE_VBLANK_MAX_AGE: Duration = Duration::from_secs(1);

/// One owed completion per CRTC, with when it became owed. A `Vec`: a
/// hotplug removes a handful of heads at a time, and entries live at most
/// [`STALE_VBLANK_MAX_AGE`].
pub(super) struct StaleVblanks {
    entries: Vec<(crtc::Handle, Instant)>,
}

impl StaleVblanks {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Records that `crtc`'s dropped head is still owed one completion.
    /// Called only where the presenter is certain an event is owed (see
    /// `Tty::stale_vblanks`); runs on the cold hotplug-removal path.
    pub(super) fn push(&mut self, crtc: crtc::Handle, now: Instant) {
        self.entries.push((crtc, now));
    }

    /// Whether the vblank just read on `crtc` is a dead head's owed event
    /// (eat it: `true`) or the live head's own (settle it: `false`).
    ///
    /// Entries older than [`STALE_VBLANK_MAX_AGE`] are dropped first: a
    /// genuine owed event is already queued at push time, so anything that
    /// old can only be a driver that never delivered. An entry exactly at
    /// the age is kept -- only strictly older ones go -- so the boundary
    /// errs toward eating, the pre-existing direction.
    ///
    /// No allocation and one timestamp compare per entry; the list is empty
    /// in steady state (entries live only between a hotplug removal and the
    /// next vblank on that CRTC), so the per-vblank cost there is nothing.
    pub(super) fn eat_stale(&mut self, crtc: crtc::Handle, now: Instant) -> bool {
        self.entries
            .retain(|(_, pushed)| now.saturating_duration_since(*pushed) <= STALE_VBLANK_MAX_AGE);
        self.entries
            .iter()
            .position(|&(dead, _)| dead == crtc)
            .is_some_and(|stale| {
                self.entries.swap_remove(stale);
                true
            })
        // (Read as find-then-remove rather than `retain`: only the first
        // vblank on this CRTC is the dead head's. A second entry for the
        // same CRTC -- two heads dropped with flips out back to back --
        // is owed a second event, so it stays for the next vblank.)
    }

    /// Forgets every owed completion: after a VT-switch reactivation or a
    /// device-wide `DrmEvent::Error` no completion can be relied on any more,
    /// and leaving an entry to eat a real vblank would freeze that screen --
    /// the worse of the two mistakes (see `Tty::stale_vblanks`).
    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::*;

    /// The CRTC the kernel would call id `raw`.
    fn crtc(raw: u32) -> crtc::Handle {
        crtc::Handle::from(NonZeroU32::new(raw).expect("crtc ids start at 1"))
    }

    const CRTC: u32 = 63;

    #[test]
    fn a_promptly_arriving_owed_event_is_eaten_as_before() {
        // The normal path, unchanged: the owed event arrives inside the
        // window, so the new head's first real vblank still settles it.
        let t0 = Instant::now();
        let mut stale = StaleVblanks::new();
        stale.push(crtc(CRTC), t0);
        assert!(stale.eat_stale(crtc(CRTC), t0 + Duration::from_millis(16)));
        // Eaten once: the next vblank on that CRTC is the live head's.
        assert!(!stale.eat_stale(crtc(CRTC), t0 + Duration::from_millis(32)));
    }

    #[test]
    fn an_entry_past_the_age_does_not_eat_the_new_heads_first_vblank() {
        // The ticket's pin: push a stale entry, advance a synthetic clock
        // past the age, then deliver a vblank for a new head on that CRTC.
        // The new head must settle -- `false` -- instead of freezing until
        // a VT switch or a DRM event error clears the list.
        let t0 = Instant::now();
        let mut stale = StaleVblanks::new();
        stale.push(crtc(CRTC), t0);
        assert!(!stale.eat_stale(
            crtc(CRTC),
            t0 + STALE_VBLANK_MAX_AGE + Duration::from_millis(1)
        ));
    }

    #[test]
    fn an_entry_exactly_at_the_age_is_still_eaten() {
        // The boundary errs toward eating, the pre-existing direction: only
        // strictly older entries age out.
        let t0 = Instant::now();
        let mut stale = StaleVblanks::new();
        stale.push(crtc(CRTC), t0);
        assert!(stale.eat_stale(crtc(CRTC), t0 + STALE_VBLANK_MAX_AGE));
    }

    #[test]
    fn mixed_ages_age_out_only_the_old_entries() {
        // Two heads dropped on different CRTCs, one owed event lost to a
        // driver fault and one arriving promptly: the vblank for the live
        // head drops the ancient entry without touching its own.
        let t0 = Instant::now();
        let mut stale = StaleVblanks::new();
        stale.push(crtc(CRTC), t0);
        stale.push(crtc(CRTC + 1), t0 + STALE_VBLANK_MAX_AGE);
        // The fresh head's vblank: its own entry is eaten...
        assert!(stale.eat_stale(
            crtc(CRTC + 1),
            t0 + STALE_VBLANK_MAX_AGE + Duration::from_millis(16)
        ));
        // ...and the ancient one aged out with it, so the other CRTC's new
        // head settles too instead of eating one more real vblank.
        assert!(!stale.eat_stale(
            crtc(CRTC),
            t0 + STALE_VBLANK_MAX_AGE + Duration::from_millis(16)
        ));
    }

    #[test]
    fn a_vblank_for_an_unknown_crtc_settles_and_keeps_fresh_entries() {
        // Every other CRTC's vblank ages the list out without eating: a
        // fresh entry for a CRTC with no vblank yet survives someone else's.
        let t0 = Instant::now();
        let mut stale = StaleVblanks::new();
        stale.push(crtc(CRTC), t0);
        assert!(!stale.eat_stale(crtc(CRTC + 1), t0 + Duration::from_millis(16)));
        assert!(stale.eat_stale(crtc(CRTC), t0 + Duration::from_millis(32)));
    }

    #[test]
    fn clearing_forgets_everything_outstanding() {
        // The VT-switch/error path: whatever is owed, live or ancient, is
        // dropped rather than left to eat a real vblank.
        let t0 = Instant::now();
        let mut stale = StaleVblanks::new();
        stale.push(crtc(CRTC), t0);
        stale.push(crtc(CRTC + 1), t0);
        stale.clear();
        assert!(!stale.eat_stale(crtc(CRTC), t0 + Duration::from_millis(16)));
        assert!(!stale.eat_stale(crtc(CRTC + 1), t0 + Duration::from_millis(16)));
    }
}
