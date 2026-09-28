//! Hiding the pointer after inactivity under a fullscreen window
//! (`[appearance] cursor_hide_after_ms`).
//!
//! On a CRTC with no cursor plane (Asahi's `apple,dcp`), a visible pointer
//! denies every fullscreen window a primary-direct attempt: the composited
//! cursor forces a composite frame. A client that hides its own pointer
//! (mpv's `--cursor-autohide`) already recovers the direct frames; this is
//! the compositor doing the same for clients that don't, and it is
//! hardware-independent: hiding only ever removes cursor elements from the
//! frame, which is what lets `render::primary_direct` judge the frame
//! eligible wherever the display would take the window.
//!
//! # What hides, and what shows again
//!
//! While a fullscreen window covers the output the pointer is on -- and the
//! pointer is over that window itself, not a popup, a layer surface or the
//! bare desktop -- `update_cursor_hide` arms a one-shot deadline
//! `cursor_hide_after_ms` out. The timer firing past the deadline hides the
//! pointer (`cursor_idle_hidden`); any pointer motion, button or scroll
//! shows it again through `note_pointer_activity`. That frame then carries
//! cursor elements, so it composites -- exactly one frame, until the next
//! deadline -- and a hidden pointer disturbs no plane assignment, because
//! there are no cursor elements for the `DrmCompositor` to place.
//!
//! Hiding is not the client's `Hidden` status: `Cursor::status` is
//! untouched, so a client-supplied image, a named shape and its hotspot all
//! survive the hide and are drawn again on the next motion. What the hide
//! suppresses is the gathering -- `State::cursor_location` answers `None`,
//! which empties both the frame's cursor elements and the capture path's
//! "where the cursor sits now", so IPC screenshots and
//! `ext-image-copy-capture-v1` read a hidden pointer as hidden (asked for
//! or not), the same way they read a client-hidden one.
//!
//! # What does not reset, and where this never hides
//!
//! - Keys never reset the deadline: typing in a fullscreen terminal is not
//!   pointer activity, and the next motion still reshows. Tablet tools do
//!   reset, through the `pointer_move`/`pointer_button` paths they already
//!   funnel through.
//! - `0` (the default) disables the feature: a compositor hiding the user's
//!   pointer unasked would be presumptuous, so this is opt-in.
//! - The lock screen never hides: locking disarms, and while locked nothing
//!   arms -- a password field needs its pointer.
//! - Every tier runs the same rule. The plane benefit is `--tty`-only, but
//!   the behavior is uniform: on `--nested` the host draws its own cursor
//!   on top either way (as it does when a client hides its own), and on
//!   `--headless` only captures can observe it.
//!
//! # Timer shape
//!
//! The motion path only writes `cursor_hide_deadline` (an `Instant` store)
//! and flips the hidden flag: no allocation, no timer churn at
//! 500-1000 Hz. The single calloop one-shot is inserted on arming -- a
//! layout/lock/VT/config transition, never per motion -- and re-arms
//! itself with `TimeoutAction::ToDuration` when it fires early (a motion
//! pushed the deadline past the scheduled firing). A firing that finds no
//! deadline is stale (a disarm it raced) and drops without hiding: only a
//! firing that still sees its own deadline past hides.
//!
//! Disarm paths all funnel through `update_cursor_hide` (called from
//! `apply()` -- so workspace switches, closes and un-fullscreens are one
//! call site -- and from popup/X-menu dismiss, which change what the
//! pointer is over with no motion and no `apply()`), from
//! `retime_cursor_hide` (config reloads), or from `disarm_cursor_hide`
//! (lock, VT pause). Unlock and VT reactivation re-evaluate through
//! `update_cursor_hide`.

use std::time::{Duration, Instant};

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::wayland::compositor::get_parent;

use super::State;
use super::output_clip::output_holds_point;

#[cfg(test)]
mod tests;

impl State {
    /// Whether the pointer qualifies for hiding right now: the feature is
    /// configured, the session is unlocked, and the pointer is over a
    /// window that covers its output fullscreen.
    ///
    /// The cover check is per output under the pointer (not merely "some
    /// window is fullscreen somewhere"), through the same predicate the
    /// scanout tier judges by (`World::fullscreen_on`), so a
    /// pointer resting on an uncovered second output never hides. Popups,
    /// layer surfaces and lock surfaces map to no window (`id_of`), so an
    /// open menu or notification over the cover correctly refuses -- those
    /// frames composite either way.
    ///
    /// Cold paths only (`update_cursor_hide` and the timer firing): a hit
    /// test plus a window-map scan per call. Never on the motion path.
    fn cursor_hide_eligible(&self) -> bool {
        if self.appearance.cursor_hide_after_ms == 0 {
            return false;
        }
        if self.session_lock.is_locked() {
            return false;
        }
        let Some(pointer) = self.seat.get_pointer() else {
            return false;
        };
        let location = pointer.current_location();
        let Some((focus, _)) = self.surface_under(location) else {
            return false;
        };
        // The hit surface's tree root: a subsurface of the covering window
        // still belongs to it (and still scans out with it), while a popup
        // or layer surface is its own root and maps to no window.
        let mut root = focus.surface().clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        let Some(id) = self.id_of(&root) else {
            return false;
        };
        // The window must cover the output the pointer is on -- a
        // fullscreen window parked on a hidden workspace covers nothing.
        // Outputs never overlap, so at most one holds the point.
        for (output_id, output) in self.outputs.iter_with_ids() {
            if self.world.fullscreen_on(output_id) != Some(id) {
                continue;
            }
            let Some(geometry) = self.space.output_geometry(output) else {
                continue;
            };
            if output_holds_point(geometry, location) {
                return true;
            }
        }
        false
    }

    /// Re-derives the hide state after anything that may have changed what
    /// covers an output: `apply()` (workspace switches, closes,
    /// un-fullscreens, output changes), popup and override-redirect-menu
    /// dismiss (which change what the pointer is over with no motion),
    /// unlock, VT reactivation, and config reloads. Takes `now` explicitly
    /// so tests drive a synthetic clock instead of sleeping for the delay.
    ///
    /// Cheap when the feature is off (one integer compare): `apply()` calls
    /// this unconditionally.
    pub(super) fn update_cursor_hide(&mut self, now: Instant) {
        if !self.cursor_hide_eligible() {
            self.disarm_cursor_hide();
            return;
        }
        if self.cursor_idle_hidden || self.cursor_hide_deadline.is_some() {
            return;
        }
        self.cursor_hide_deadline = Some(now + self.cursor_hide_delay());
        self.arm_cursor_hide_timer();
    }

    /// Pointer activity: motion, button or scroll. Shows a hidden pointer
    /// again (through the ordinary cursor redraw, so that frame carries
    /// the cursor and composites) and pushes the deadline out.
    ///
    /// Hot path: one integer compare while the feature is off, else one
    /// clock read, two stores, and no allocation -- the timer is inserted
    /// only when none is live. Deliberately not called from the quiet
    /// re-derivations (`pointer_move_quietly`): the compositor re-running
    /// its own hit test is not a user at the machine. Keys never call it
    /// either (see the module doc).
    pub(super) fn note_pointer_activity(&mut self) {
        if self.appearance.cursor_hide_after_ms == 0 {
            return;
        }
        if self.session_lock.is_locked() {
            return;
        }
        if self.cursor_idle_hidden {
            self.cursor_idle_hidden = false;
            // The reshown cursor's own redraw: that frame carries cursor
            // elements again, so it composites -- exactly one, until the
            // next deadline.
            self.cursor_changed();
        }
        self.cursor_hide_deadline = Some(Instant::now() + self.cursor_hide_delay());
        self.arm_cursor_hide_timer();
    }

    /// Re-derives the hide state after the configured delay itself changed
    /// (config reload): unlike `update_cursor_hide`, an armed deadline
    /// computed under the old delay is stale, so an eligible, still-visible
    /// session is re-timed to `now +` the new delay -- shortening pulls the
    /// hide in, lengthening pushes it out, disabling disarms. A hidden
    /// session stays hidden (a delay change is not activity), and an
    /// ineligible one disarms exactly as in `update_cursor_hide`.
    pub(super) fn retime_cursor_hide(&mut self, now: Instant) {
        if !self.cursor_hide_eligible() {
            self.disarm_cursor_hide();
            return;
        }
        if !self.cursor_idle_hidden {
            self.cursor_hide_deadline = Some(now + self.cursor_hide_delay());
            self.arm_cursor_hide_timer();
        }
    }

    /// Forgets any armed deadline and shows the pointer again: the lock and
    /// VT-pause half of the disarm paths (`apply()` covers the rest through
    /// `update_cursor_hide`). A firing that already escaped sees no
    /// deadline and drops without hiding.
    pub(super) fn disarm_cursor_hide(&mut self) {
        self.cursor_hide_deadline = None;
        if self.cursor_idle_hidden {
            self.cursor_idle_hidden = false;
            self.cursor_changed();
        }
    }

    /// The one-shot timer's firing: hides past the deadline, re-arms for
    /// the remainder when a motion pushed it, and drops everywhere else.
    /// Takes `now` explicitly (the timer's own firing time at runtime), so
    /// tests run the race deterministically.
    pub(super) fn note_cursor_hide_timeout(&mut self, now: Instant) -> TimeoutAction {
        if !self.cursor_hide_eligible() {
            // The hide arm is the only writer of `true`, and it drops the
            // timer in the same statement -- so a live timer firing here
            // always finds `false`. The assert pins that for future edits:
            // clearing a set flag here without redrawing would leave the
            // pointer invisibly hidden until the next motion.
            debug_assert!(
                !self.cursor_idle_hidden,
                "a hidden pointer outlived its cover without a disarm"
            );
            self.cursor_hide_deadline = None;
            self.cursor_hide_timer_live = false;
            return TimeoutAction::Drop;
        }
        match self.cursor_hide_deadline {
            // A motion pushed the deadline past this firing (the timeout
            // race): keep the same timer, fire again at the new deadline.
            Some(deadline) if now < deadline => {
                self.cursor_hide_timer_live = true;
                TimeoutAction::ToDuration(deadline - now)
            }
            // A disarm this firing raced: no deadline, no hide.
            None => {
                self.cursor_hide_timer_live = false;
                TimeoutAction::Drop
            }
            // At (or past) the deadline, still covered: hide. The timer
            // drops -- nothing left to wait for -- and the next motion
            // re-arms through `note_pointer_activity`.
            Some(_) => {
                self.cursor_hide_deadline = None;
                self.cursor_hide_timer_live = false;
                self.cursor_idle_hidden = true;
                // The hidden cursor's own redraw: the frame without it is
                // what may go primary-direct, and captures re-serve without
                // it (`cursor_serial` moves even where frames draw none).
                self.cursor_changed();
                TimeoutAction::Drop
            }
        }
    }

    /// The configured delay as a `Duration`. Only read where the feature
    /// is already known enabled (`cursor_hide_eligible` or the zero check
    /// in `note_pointer_activity`), so no `Duration::ZERO` edge: a zero
    /// never reaches a timer.
    fn cursor_hide_delay(&self) -> Duration {
        Duration::from_millis(self.appearance.cursor_hide_after_ms)
    }

    /// Inserts the one-shot hide timer, unless one is already live. Arming
    /// (not firing) is what allocates an event source, so this runs on
    /// transitions, never per motion.
    fn arm_cursor_hide_timer(&mut self) {
        if self.cursor_hide_timer_live {
            return;
        }
        self.cursor_hide_timer_live = true;
        if let Err(error) = self.loop_handle.insert_source(
            Timer::from_duration(self.cursor_hide_delay()),
            cursor_hide_timeout,
        ) {
            // Without the timer the deadline never fires: drop it, so the
            // next transition or motion retries the arm rather than leaving
            // a deadline nothing will ever serve.
            tracing::error!(%error, "could not arm the cursor-hide timer");
            self.cursor_hide_timer_live = false;
            self.cursor_hide_deadline = None;
        }
    }
}

/// The calloop one-shot's callback: [`State::note_cursor_hide_timeout`]
/// with the timer's own firing time.
fn cursor_hide_timeout(now: Instant, _: &mut (), state: &mut State) -> TimeoutAction {
    state.note_cursor_hide_timeout(now)
}
