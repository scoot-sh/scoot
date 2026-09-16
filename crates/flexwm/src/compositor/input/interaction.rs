//! The serials of the input events that count as "the user just did
//! something".
//!
//! One consumer today: `xdg-activation-v1`. The protocol lets a client
//! attach the seat and serial of the input event that caused it to ask for a
//! token (`xdg_activation_token_v1.set_serial`), and the compositor is meant
//! to refuse a token whose serial does not name a real, recent interaction
//! -- that is the only thing standing between "a launcher hands focus to the
//! app it just started" and "any background client takes the keyboard
//! whenever it likes". See `activation.rs` for the policy this feeds.
//!
//! # Why a ring rather than "the last serial"
//!
//! A single remembered serial cannot answer the question correctly, in
//! either direction:
//!
//! - A keyboard-driven launcher acts on the *press* and mints its token
//!   from that serial -- but `State::press` sends the press and the matching
//!   release back to back within one call, with no dispatch in between, so
//!   by the time the client's `commit` is handled the last serial issued is
//!   the *release*'s. (`flexwm msg key Return` is exactly this, and it is
//!   how the real-launcher flow in
//!   `docs/backlog/resolved/foot-protocol-warnings-done.md` was driven.)
//! - A pointer-driven one (a GTK button activates on release, not press)
//!   mints from the release serial, which a press-only tracker never saw.
//!
//! So what has to be remembered is a short *history* of qualifying serials,
//! and a token is honored if its serial is any one of them. Exact match
//! against remembered values, never a range: serials from events that do
//! *not* qualify (pointer motion) and from the compositor's own focus
//! changes (`shell.rs`'s `set_focus`) are drawn from the same global
//! counter and fall between them, so "between the oldest and the newest"
//! would quietly admit the passive events this exists to exclude.
//!
//! Fixed-size and stored inline in [`State`], with no heap indirection of
//! its own: this is written from the input path, on every key and button
//! event, and must not allocate.
//!
//! [`State`]: super::State

use smithay::utils::Serial;

/// How many qualifying serials are remembered.
///
/// Sized off the largest burst one user action can produce, so that the
/// serial a client legitimately captured cannot be evicted before the
/// client's own request makes it back to the compositor: a full four-modifier
/// combo through [`State::press`] is ten events (four modifier presses, the
/// key's press and release, four modifier releases), and `type_text` spends
/// two to six per character. Sixteen covers that with room to spare while
/// staying a fraction of a second of real typing -- which is the other half
/// of the bound, since every remembered serial is one a token may still be
/// minted against.
///
/// `pub(crate)` only so `activation/tests.rs` can say "older than the whole
/// history" without hard-coding a number that would silently stop meaning
/// that if this changed.
///
/// [`State::press`]: super::State::press
pub(crate) const CAPACITY: usize = 16;

/// The most recent qualifying input serials, newest overwriting oldest.
#[derive(Debug)]
pub(crate) struct Recent {
    /// `None` only before the ring has been filled once -- a fresh session
    /// that has seen no input at all matches nothing, which is the case the
    /// activation gate exists for.
    serials: [Option<Serial>; CAPACITY],
    /// Where the next serial goes. Always `< CAPACITY` (see [`Self::record`]),
    /// so indexing with it cannot panic.
    next: usize,
}

impl Default for Recent {
    fn default() -> Self {
        Self {
            serials: [None; CAPACITY],
            next: 0,
        }
    }
}

impl Recent {
    /// Remembers one serial, evicting the oldest once full.
    pub(crate) fn record(&mut self, serial: Serial) {
        self.serials[self.next] = Some(serial);
        self.next = (self.next + 1) % CAPACITY;
    }

    /// Whether `serial` is one of the remembered ones.
    ///
    /// A linear scan of sixteen `Option<u32>`s, on a path that runs once per
    /// activation token (a user action), not per event.
    pub(crate) fn contains(&self, serial: Serial) -> bool {
        self.serials.contains(&Some(serial))
    }

    /// The serial recorded most recently, if any.
    ///
    /// Tests only: the compositor itself never asks "what was the last one",
    /// precisely because that question has no single right answer (see the
    /// module doc). Tests need it to learn the serial a simulated press was
    /// sent with, which is otherwise unobservable -- [`SERIAL_COUNTER`] is
    /// process-global and shared with every other test running in parallel.
    ///
    /// [`SERIAL_COUNTER`]: smithay::utils::SERIAL_COUNTER
    #[cfg(test)]
    pub(crate) fn latest(&self) -> Option<Serial> {
        let last = (self.next + CAPACITY - 1) % CAPACITY;
        self.serials[last]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serial(raw: u32) -> Serial {
        Serial::from(raw)
    }

    #[test]
    fn a_fresh_ring_remembers_nothing() {
        let recent = Recent::default();
        assert!(!recent.contains(serial(1)));
        assert_eq!(recent.latest(), None);
    }

    #[test]
    fn a_recorded_serial_is_remembered() {
        let mut recent = Recent::default();
        recent.record(serial(7));
        assert!(recent.contains(serial(7)));
        assert!(!recent.contains(serial(8)));
        assert_eq!(recent.latest(), Some(serial(7)));
    }

    #[test]
    fn every_serial_in_a_full_ring_still_matches() {
        // The press/release pairs of one burst must all stay valid: which of
        // them a given client minted its token from is the client's choice,
        // not something this can predict.
        let mut recent = Recent::default();
        for raw in 1..=CAPACITY as u32 {
            recent.record(serial(raw));
        }
        for raw in 1..=CAPACITY as u32 {
            assert!(recent.contains(serial(raw)), "{raw} was forgotten early");
        }
    }

    #[test]
    fn the_oldest_serial_is_evicted_once_the_ring_wraps() {
        let mut recent = Recent::default();
        for raw in 1..=CAPACITY as u32 + 1 {
            recent.record(serial(raw));
        }
        assert!(!recent.contains(serial(1)), "the oldest serial survived");
        assert!(recent.contains(serial(2)));
        assert!(recent.contains(serial(CAPACITY as u32 + 1)));
        assert_eq!(recent.latest(), Some(serial(CAPACITY as u32 + 1)));
    }

    #[test]
    fn recording_far_past_the_capacity_keeps_exactly_the_last_capacity_serials() {
        // The index arithmetic wraps rather than growing, so a long session
        // is the same memory and the same cost as a fresh one.
        let mut recent = Recent::default();
        for raw in 1..=1_000u32 {
            recent.record(serial(raw));
        }
        assert!(!recent.contains(serial(1_000 - CAPACITY as u32)));
        assert!(recent.contains(serial(1_000 - CAPACITY as u32 + 1)));
        assert!(recent.contains(serial(1_000)));
    }

    #[test]
    fn a_wrapped_around_counter_is_matched_by_value() {
        // `Serial`'s own ordering is wrap-aware, but this only ever asks for
        // equality, so a counter that has wrapped past `u32::MAX` matches the
        // same way any other value does.
        let mut recent = Recent::default();
        recent.record(serial(u32::MAX));
        recent.record(serial(0));
        recent.record(serial(1));
        assert!(recent.contains(serial(u32::MAX)));
        assert!(recent.contains(serial(0)));
        assert!(!recent.contains(serial(2)));
    }
}
