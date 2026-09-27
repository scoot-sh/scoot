//! Re-constraining `reactive` popups after the conditions move under them.
//!
//! `popup_constraint.rs` constrains a popup against its output at the two
//! points the client is told its geometry: the initial configure and
//! `xdg_popup.reposition`. That leaves the case `xdg_positioner.set_reactive`
//! (v3) names: "the surface is reconstrained if the conditions used for
//! constraining changed, e.g. the parent window moved", answered with a fresh
//! `xdg_popup.configure` + `xdg_surface.configure` pair. A menu slid inside
//! the output at open time would otherwise end up cut again once its column
//! scrolls, the output is resized, or a bar's exclusive zone changes -- while
//! a non-reactive popup is correctly left alone (the protocol forbids
//! re-configuring one: Smithay answers it with `NotReactive`).
//!
//! # When it runs
//!
//! At the end of [`State::apply`], which is the one choke point every change
//! of the conditions reaches: layout actions, window map/unmap, output
//! resize and add/remove (all of which re-`apply`), and layer-surface
//! commits (whose exclusive-zone edits re-`apply` through
//! `commit_layer_surface`). Running there rather than at each call site keeps
//! the rule in one place, and running after the arrangement is published
//! means the target walk reads where the parent *is*, not where it was.
//!
//! # How it composes with the deferred commits
//!
//! `flush_window_commits` (`window_commit.rs`) coalesces a dispatch's bbox
//! recomputes and `apply()`s once when a frame moved the layout -- and this
//! pass runs inside that same `apply()`, after the windows are mapped. A
//! popup's own commit therefore never meets a half-moved parent: either the
//! parent's commit batch already applied (and the re-constrain below already
//! sent whatever it owed), or it has not, and the flush's `apply()` sends it
//! now. The pass only ever *sends* configures; it never consumes a commit,
//! so there is nothing for the coalescing to reorder against.
//!
//! # What is re-constrained, and what is left alone
//!
//! For each tracked `xdg_popup` (scoot's own index, so the common
//! popup-less `apply()` pays one length check):
//!
//! - unmapped popups (no initial configure yet), dead surfaces, and popups
//!   dismissed out of their tree (a refused grab tears the node down while
//!   the object lives on) are skipped -- the membership half mirrors
//!   `send_popup_initial_configure`'s, for the same reason;
//! - a popup whose committed positioner is not `reactive` is skipped: a
//!   re-configure there is a protocol error, not a courtesy;
//! - a popup whose positioner is newer than what the client acked (a
//!   `reposition` in flight) is skipped: its newer geometry is already on
//!   the wire, and this pass must not write an older positioner's answer
//!   over it. The next `apply()` after the ack picks it up;
//! - otherwise the target is recomputed (`constrained_popup_geometry`) and,
//!   when it differs from what is already pending-or-acked, the pending
//!   geometry is set and `send_configure()` goes out. Comparing against the
//!   pending-seeded state (which Smithay seeds from the last *sent*
//!   configure when one is unacked) rather than only the committed one also
//!   keeps a second `apply()` before the ack from sending a duplicate.
//!
//! A `send_configure` that still fails (an old client that cannot be
//! re-configured at all) is logged and left: the next `apply()` tries
//! again, and nothing retries specially.
//!
//! # Grabs
//!
//! A popup grab routes *input*; a configure carries *geometry*. The two do
//! not fight: a menu opened with a grab and then scrolled past by an IPC
//! action is re-constrained like any other reactive popup, the grab still
//! held, no `popup_done`. The grab tests pin that half explicitly.
//!
//! # Cost
//!
//! One length check per `apply()` with no popup open. With popups open, one
//! index walk plus per popup: liveness and role checks already paid on the
//! commit path, a bounded per-tree membership walk, and a handful of surface
//! state reads -- no allocation past the index walk's own handle clones
//! (one per live popup, the same clone the configure lookup performs).

use smithay::desktop::{PopupKind, PopupManager, find_popup_root_surface};

use super::State;

impl State {
    /// Re-constrains every tracked reactive popup whose conditions changed;
    /// see the module doc. Called at the end of [`State::apply`].
    pub(super) fn reconstrain_reactive_popups(&self) {
        if self.popup_index.is_empty() {
            return;
        }
        // Cloned out first: the walk below reads surface state (which takes
        // other surfaces' locks) and sends configures, none of which may
        // happen while the index itself is borrowed.
        let popups: Vec<PopupKind> = self.popup_index.iter().collect();
        for kind in &popups {
            self.reconstrain_popup(kind);
        }
    }

    /// One popup's share of [`State::reconstrain_reactive_popups`].
    fn reconstrain_popup(&self, kind: &PopupKind) {
        let PopupKind::Xdg(popup) = kind else {
            return;
        };
        if !popup.alive() || !popup.is_initial_configure_sent() {
            return;
        }
        // Dismissed out of its tree while the object lives on: not
        // configured, exactly as the initial-configure path treats it.
        let member = find_popup_root_surface(kind)
            .map(|root| {
                PopupManager::popups_for_surface(&root)
                    .any(|(kind, _)| kind.wl_surface() == popup.wl_surface())
            })
            .unwrap_or(false);
        if !member {
            return;
        }
        // The committed pair: the last geometry the client acked, and the
        // positioner it goes with. `None` only for a mapped popup that
        // never acked, which the protocol does not allow (no buffer without
        // acking the initial configure) -- nothing to re-constrain against.
        let Some(committed) = popup.with_committed_state(|state| state.cloned()) else {
            return;
        };
        if !committed.positioner.reactive {
            return;
        }
        // Seeded from the last *sent* configure when one is unacked, else
        // the committed state: both what to recompute from (a `reposition`
        // in flight must not be overwritten with an older positioner's
        // answer) and what to compare against (an unacked re-constrain must
        // not be sent twice).
        let (pending_positioner, pending_geometry) =
            popup.with_pending_state(|state| (state.positioner, state.geometry));
        if pending_positioner != committed.positioner {
            return;
        }
        let Some(recomputed) = self.constrained_popup_geometry(popup, committed.positioner) else {
            return;
        };
        if recomputed == pending_geometry {
            return;
        }
        popup.with_pending_state(|state| {
            state.geometry = recomputed;
        });
        if let Err(error) = popup.send_configure() {
            tracing::debug!(?error, "reactive popup re-constrain failed");
        }
    }
}
