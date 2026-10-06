//! Bind repeat and per-bind allow-when-locked.
//!
//! Two per-bind opt-ins that both ride on the keybinding dispatch in
//! `input.rs::key` (see `keybindings::BindFlags` for the flags themselves,
//! `config.rs::parse_bind` for the file shape):
//!
//! - **`repeat`**: holding the bind's key re-fires its action, at the seat
//!   keyboard's own repeat delay and rate -- the same `200 ms` / `25/s` the
//!   session hands clients through `wl_keyboard.repeat_info` (see
//!   `KEYBOARD_REPEAT_DELAY_MS` / `KEYBOARD_REPEAT_RATE_PER_SECOND`, shared
//!   with `State::new`'s `add_keyboard` so the two cannot disagree).
//!   Compositor-side repetition is needed at all because Smithay absorbs a
//!   press of an already-held key before the bind filter (the pinned fork's
//!   `KbdInternal::key_input`: "don't double-run the filter"), so a kernel
//!   repeat never reaches the filter and a held bind would fire exactly
//!   once. That same absorption is why clients are unaffected: they repeat
//!   client-side off `repeat_info`, which this session has always sent.
//! - **`allow_when_locked`**: a `spawn` bind fires while the session is
//!   locked (volume, brightness, media -- the desktop profile's keymap opts
//!   exactly those in). Every other bind stays refused at all three gates,
//!   exactly as now: the key filter forwards the keystroke to the lock
//!   client, `State::act` refuses everything while locked, and IPC
//!   `Request::Action` stays refused (see below).
//!
//! # Why an allowlist of one action shape
//!
//! An allowed bind can only run its config-pinned command: the key path
//! looks the bind up in the session's own table, so a locked session can
//! never be made to launch anything but the opted-in commands. An IPC
//! `action` request, by contrast, carries an arbitrary command from the
//! requester -- allowing it while locked would turn "volume keys work on
//! the lock screen" into "anything with socket access runs anything while
//! locked". It stays refused, deliberately, and `State::act` (the backstop
//! every other caller reaches) refuses everything while locked: only the
//! keybinding path (`State::act_bind`) may let an opted-in `spawn`
//! through, and it checks the action shape itself rather than trusting the
//! flag.
//!
//! # Timer shape
//!
//! One in-flight repeat at most (the latest repeatable press wins), armed
//! only while such a key is held: the calloop one-shot is inserted on the
//! press and drops itself on release or any other cancel path, so a session
//! with no held repeatable key has no timer source and no idle wakeups.
//! The first re-fire waits the full delay; later ones step at the rate. All
//! timing runs on explicit `Instant`s (`arm_bind_repeat` takes `now`, the
//! timer hands its own firing time to `note_bind_repeat_timeout`), so tests
//! drive a synthetic clock instead of sleeping.
//!
//! Cancel paths, each load-bearing: the key's release (or any path that
//! releases it, e.g. `--nested`'s host-keyboard leave via
//! `release_held_keys`), a VT switch attempt, every lock transition
//! (`lock_transition`, which every lock-side change funnels through, plus
//! `unlock`), a `--tty` session pause, an output removal, and a `[binds]`
//! reload that swaps the table (the in-flight action may be gone, or its
//! flags changed). Keyboard *focus* moves do not cancel: holding volume
//! while a window maps keeps stepping, which is the point.

use std::time::{Duration, Instant};

use scoot_core::Action;
use smithay::input::keyboard::Keycode;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use super::State;

#[cfg(test)]
mod tests;

/// The seat keyboard's repeat delay, in milliseconds: how long a key is
/// held before the first re-fire. Shared with `State::new`'s
/// `add_keyboard`, which hands the same pair to clients through
/// `wl_keyboard.repeat_info` -- see the module doc.
pub(crate) const KEYBOARD_REPEAT_DELAY_MS: i32 = 200;
/// The seat keyboard's repeat rate, in presses per second: the steady-state
/// step while a repeatable bind's key stays held. Shared the same way.
pub(crate) const KEYBOARD_REPEAT_RATE_PER_SECOND: i32 = 25;

/// How long a repeatable press waits before its first re-fire.
pub(super) fn repeat_delay() -> Duration {
    Duration::from_millis(KEYBOARD_REPEAT_DELAY_MS as u64)
}

/// The steady-state step between re-fires. Exact: the rate divides the
/// second evenly, so no rounding accumulates a drift.
pub(super) fn repeat_interval() -> Duration {
    Duration::from_millis(1000 / KEYBOARD_REPEAT_RATE_PER_SECOND as u64)
}

/// Whether this action may re-fire while its bind's key is held. Only the
/// flag opts in (see `keybindings::BindFlags`), and even then `quit`,
/// `close` and `show-keymap` never repeat: holding quit must never end the
/// session, holding close must never work through every window, and holding
/// the keymap key must never stack terminals. `config.rs` warns
/// and clears the flag on those at load; this is the backstop that holds
/// even for a table built any other way (a test, a future caller).
pub(super) fn bind_action_repeats(action: &Action) -> bool {
    !matches!(
        action,
        Action::Quit | Action::CloseFocused | Action::ShowKeymap
    )
}

/// The in-flight bind repeat, if a `repeat` bind's key is held: what to
/// re-fire, and when. At most one (see the module doc).
#[derive(Clone, Debug)]
pub(crate) struct BindRepeat {
    /// The held keycode this repeat belongs to: its release cancels it.
    /// Keycodes, not combos -- modifiers held around the press stay held
    /// across the whole repeat, and only this key's own release ends it.
    pub keycode: Keycode,
    /// The bound action, cloned at press time: re-firing never re-reads
    /// the table, so a reload swapping the table mid-hold cannot change
    /// what repeats (the reload cancels the repeat outright instead --
    /// see `reload.rs`).
    pub action: Action,
    /// Whether the press was allowed while locked: the timer's re-fires go
    /// through the same locked-aware path as the press, so a repeat armed
    /// before a lock cannot outlive it (the lock cancels it first) and one
    /// armed while locked keeps stepping while held.
    pub allow_when_locked: bool,
    /// When the next re-fire is due: press time plus the delay for the
    /// first, then stepping at the rate.
    pub next: Instant,
}

impl State {
    /// Arms the in-flight repeat for a repeatable press, replacing any
    /// previous one (the latest press wins -- see the module doc), and
    /// inserts the one-shot timer unless one is already live.
    ///
    /// Takes `now` explicitly so tests drive a synthetic clock. The hot
    /// path cost is paid only for opted-in binds: one `Action` clone (the
    /// command `Vec`, so no clone for non-`spawn` repeats) plus, at most
    /// once per hold, one event-source insert.
    pub(super) fn arm_bind_repeat(
        &mut self,
        keycode: Keycode,
        action: Action,
        allow_when_locked: bool,
        now: Instant,
    ) {
        self.bind_repeat = Some(BindRepeat {
            keycode,
            action,
            allow_when_locked,
            next: now + repeat_delay(),
        });
        if self.bind_repeat_timer_live {
            return;
        }
        self.bind_repeat_timer_live = true;
        if let Err(error) = self
            .loop_handle
            .insert_source(Timer::from_duration(repeat_delay()), bind_repeat_timeout)
        {
            // Without the timer the repeat never fires: drop it, so the
            // state never claims a repeat is armed that nothing serves.
            tracing::error!(%error, "could not arm the bind-repeat timer");
            self.bind_repeat_timer_live = false;
            self.bind_repeat = None;
        }
    }

    /// Forgets any in-flight repeat: the release path, and every cancel path
    /// in the module doc. The live timer, if any, fires once more, finds
    /// nothing, and drops -- so cancellation itself never touches the event
    /// loop. One `Option` write; safe to call unconditionally on cold paths.
    pub(super) fn cancel_bind_repeat(&mut self) {
        self.bind_repeat = None;
    }

    /// Forgets the in-flight repeat when the released key is the one it
    /// belongs to. Releases of any other key (a modifier let go mid-hold,
    /// another bind's key) leave it alone.
    pub(super) fn cancel_bind_repeat_for(&mut self, keycode: Keycode) {
        if self
            .bind_repeat
            .as_ref()
            .is_some_and(|repeat| repeat.keycode == keycode)
        {
            self.bind_repeat = None;
        }
    }

    /// The one-shot timer's firing: re-fires past the deadline, re-arms
    /// for the remainder when it fired early, and drops everywhere else.
    /// Takes `now` explicitly (the timer's own firing time at runtime), so
    /// tests run the timing deterministically.
    pub(super) fn note_bind_repeat_timeout(&mut self, now: Instant) -> TimeoutAction {
        let Some(repeat) = self.bind_repeat.clone() else {
            // A cancel this firing raced: no repeat, no fire.
            self.bind_repeat_timer_live = false;
            return TimeoutAction::Drop;
        };
        if now < repeat.next {
            // Fired early (the timer was armed for the delay and the
            // deadline moved, or a second arm reused the live timer):
            // keep the same source, fire again at the deadline.
            self.bind_repeat_timer_live = true;
            return TimeoutAction::ToDuration(repeat.next - now);
        }
        // At (or past) the deadline, still held: re-fire through the same
        // locked-aware path as the press, then step the deadline at the
        // rate. The deadline steps from `now`, not from itself: a late
        // firing (the loop was busy) does not bunch up catch-up fires.
        // Written back before firing, so a re-entrant cancel (nothing in
        // `act_bind` cancels today, but the invariant should not depend
        // on that) cannot resurrect a stale deadline.
        if let Some(live) = self.bind_repeat.as_mut() {
            live.next = now + repeat_interval();
        }
        self.bind_repeat_timer_live = true;
        self.act_bind(repeat.action, repeat.allow_when_locked);
        TimeoutAction::ToDuration(repeat_interval())
    }
}

/// The calloop one-shot's callback: [`State::note_bind_repeat_timeout`]
/// with the timer's own firing time.
fn bind_repeat_timeout(now: Instant, _: &mut (), state: &mut State) -> TimeoutAction {
    state.note_bind_repeat_timeout(now)
}
