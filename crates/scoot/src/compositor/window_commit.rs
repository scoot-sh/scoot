//! Window bbox recomputes, coalesced to one per dispatch per window.
//!
//! # Why the recompute cannot stay per commit
//!
//! `Window::on_commit` recomputes the window's bounding box over its whole
//! surface tree (`bbox_from_surface_tree` at the pinned rev,
//! `src/desktop/wayland/utils.rs`): linear in the tree per call. A
//! desynchronized subsurface's commit applies at once, and scoot's commit
//! handler used to recompute the window on every one -- so a client mapping
//! a window and then committing `N` 1-deep desynchronized siblings in one
//! batch paid linear per commit, quadratic per batch (see
//! `docs/backlog/core/subsurface-count-quadratic.md`: 1000 for 28 ms, 3000
//! for 84 ms, 10000 for 1.15 s in release on the dev VM, 30000 past the
//! harness timeout). A synchronized child's commit is only cached until its
//! parent's, which is why that case was always cheap. A per-client count cap
//! -- the shape of the sibling popup/toplevel bounds -- does not fit here:
//! thousands of side-by-side subsurfaces are ordinary client behaviour (a
//! browser's compositor layers), so the fix coalesces instead of bounding.
//!
//! # What is deferred, and what is not
//!
//! Only the bbox recompute itself is deferred: `commit()` records the
//! window ([`State::note_window_commit`]) and the recompute runs once per
//! window after the batch ([`State::flush_window_commits`]), called from
//! the Wayland display source right after `dispatch_clients` -- beside
//! `refresh_lock_state` and `settle_popup_grab`, which are the same pattern
//! (per-dispatch work factored out of a per-request handler) -- so every
//! other event source in the loop still sees either the pre-batch bbox or
//! the flushed one, exactly as it did when the recompute ran per commit.
//!
//! Everything else in the commit tail stays per commit, because a client can
//! observe all of it: damage (`request_render`), the sync state
//! (`on_commit_buffer_handler`, untouched), the initial configure, the
//! map-time floating decision (which reads toplevel role data, not the
//! bbox) and the fullscreen-discard check (a world flag plus a
//! configure-sent flag, not the bbox).
//!
//! `observe_frame` moves to the flush with the recompute, rather than
//! staying per commit, because it is the one per-commit reader of the bbox
//! (`window.geometry().size`) and a per-commit read can only ever repeat
//! the flush's or precede it:
//!
//! - Unchanged box (a damage-only recommit, an ack with no new pixels):
//!   the same requested size against the same actual one the last flush
//!   already reported -- learning takes a maximum, `drawn` is rewritten
//!   with the same value, the floating request is overwritten with the
//!   same fit -- an exact no-op.
//! - Changed box: the pre-batch size paired with this commit's acked
//!   configure. The core's learned minimums only ever grow, so a shrink
//!   answered mid-batch would stick a bogus minimum -- caught live by
//!   `fullscreen::tests::transitions`, where a late output-sized frame
//!   after leaving fullscreen widened the column for good. Nothing
//!   consumes the intermediate values mid-dispatch (the core is only read
//!   by `apply()`, which no protocol handler in the batch runs), so
//!   dropping them loses nothing and the flush's observe -- fresh box,
//!   post-batch acked state -- is the same event the last commit's own
//!   observe would have sent had the recompute run per commit.
//!
//! The one residual staleness is a handler in the *same* batch reading the
//! bbox for another purpose -- positioning an input-method popup
//! (`input_method.rs`) or constraining one (`popup_constraint.rs`) when the
//! client interleaves those requests with subsurface commits in one flush.
//! That computation sees the pre-batch box and heals at the flush, with no
//! protocol error either way; damage, frame callbacks and sync state are
//! unaffected.
//!
//! # Cost
//!
//! The pending set is a pooled `Vec`, pushed only when the window is not
//! already in it: distinct dirty windows per dispatch are bounded by the
//! window count (itself per-client capped), so the scan is trivial, and
//! `clear()` keeps the capacity -- no per-commit or per-dispatch heap past
//! the first batch that dirtied that many windows.
//!
//! # Scope
//!
//! X commits flow through the same `commit()` tail (`id_of`'s X arm), so an
//! X window's recompute is coalesced the same way; there is no second path
//! that needs the treatment. Subsurface trees under layer surfaces and lock
//! surfaces never reach this code (no window owns them), so they never
//! paid the recompute and still do not.

use scoot_core::WindowId;

use super::State;

impl State {
    /// Records that `id`'s window needs its bbox recomputed before anything
    /// else in the loop can observe it -- the deferred half of what
    /// `Window::on_commit` did per commit. Idempotent within a dispatch: a
    /// window flooded by `N` commits is recorded once.
    pub(super) fn note_window_commit(&mut self, id: WindowId) {
        if !self.pending_window_commits.contains(&id) {
            self.pending_window_commits.push(id);
        }
    }

    /// Recomputes every window [`State::note_window_commit`] recorded, in
    /// first-dirtied order, then re-reports each one's frame so the core's
    /// last word carries the fresh size (see the module doc). Hands back
    /// how many windows were recomputed.
    ///
    /// A window closed between its commit and this flush -- a client can
    /// destroy the toplevel later in the same batch -- is skipped: there is
    /// no bbox left to recompute, and `observe_frame` would early-return on
    /// it anyway.
    ///
    /// Runs once per dispatch, never per commit; an empty pending set is
    /// one length check. Single-threaded like the rest of dispatch: nothing
    /// else marks windows while this drains, but the cursor form is used
    /// anyway so a mark from anywhere could only add work, never lose it.
    pub(super) fn flush_window_commits(&mut self) -> usize {
        let mut recomputed = 0;
        let mut index = 0;
        while index < self.pending_window_commits.len() {
            let id = self.pending_window_commits[index];
            index += 1;
            {
                let Some(window) = self.window(id) else {
                    continue;
                };
                window.on_commit();
            }
            self.observe_frame(id);
            recomputed += 1;
        }
        self.pending_window_commits.clear();
        #[cfg(test)]
        {
            self.last_window_commit_flush = recomputed;
        }
        recomputed
    }
}

#[cfg(test)]
mod tests;
