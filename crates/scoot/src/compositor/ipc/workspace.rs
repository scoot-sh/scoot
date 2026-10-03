//! Workspace occupancy events: which workspaces hold windows, per output.
//!
//! A `workspace` subscription carries [`WorkspaceSnapshot`](scoot_ipc::WorkspaceSnapshot)
//! whenever an output's snapshot moves -- a window opened, closed or moved
//! between workspaces, or the active workspace switched -- so a bar can dim
//! the empty workspaces without polling `windows`, and an agent learns
//! "workspace 3 now has windows" the same way. Each event is a **full
//! snapshot** of its output rather than a delta, so a subscriber that
//! missed one is never wrong.
//!
//! This rides the same choke point as `refresh_workspaces` (the only writer
//! of the ext-workspace state): every workspace change passes through
//! `State::apply`, whose workspace refresh ends here. But occupancy is a
//! snapshot of its own, not piggybacked on that diff -- moving a window
//! from workspace 0 to workspace 1 changes no workspace count and no
//! active index, so the ext-workspace compare would never fire for it.
//!
//! The cost rules, matching the ticket's bounds:
//!
//! - **Coalesced** to at most one event per output per frame tick. A change
//!   only *marks* the output's snapshot dirty; the tick carries the latest
//!   (`State::flush_workspace_events`, before the subscriber tails drain),
//!   so a client opening and closing windows at its maximum rate is one
//!   event per tick, not an event stream. A change that reverts before the
//!   tick unmarks, and carries nothing.
//! - **Cheap with no subscriber**: one walk of the subscriber list, and
//!   nothing is read off the core and nothing built. Skipped reads while
//!   unsubscribed leave the published record behind, which is why
//!   subscribing syncs it without emitting instead of reporting the whole
//!   unsubscribed interval as one change (the keyboard detector's half has
//!   the same shape -- see `input.rs`'s `check_keyboard_layout`).
//! - **Allocation-free on the send path**: the per-output counts are built
//!   in a reused scratch buffer, compared against the published snapshot in
//!   place, and a changed snapshot is swapped into its pending slot rather
//!   than copied. The tick encodes once per changed output and the shared
//!   emit tail carries that line to every subscriber.
//!
//! The backpressure policy is the shared one (see `events.rs`): a
//! subscriber that stops reading is disconnected past the 1 MiB mark or
//! the stall window, never buffered without bound, and a part-written tail
//! drains on the tick. A fresh subscription starts silent -- read `windows`
//! once for the baseline, then apply snapshots after it -- so the first
//! event after subscribing is a real change, not the backlog.

use scoot_core::OutputId;
use scoot_ipc::{EventKind, Response, WorkspaceSnapshot, encode};

use super::State;

impl State {
    /// Whether any subscriber wants occupancy snapshots -- the gate every
    /// refresh takes before reading the core at all. One short walk of the
    /// subscriber list, and nothing is built without one.
    pub(super) fn wants_workspace_events(&self) -> bool {
        self.subscribers
            .iter()
            .any(|subscriber| subscriber.wants(EventKind::Workspace))
    }

    /// Re-reads every output's occupancy snapshot and marks what moved.
    ///
    /// Called at the end of `State::refresh_workspaces`, which is where
    /// every workspace change already diffs -- window opens/closes/moves
    /// through `State::apply`, output add/remove through the same call.
    /// Only marks: the frame tick carries the marked snapshots out, at
    /// most one event per output per tick.
    pub(crate) fn refresh_workspace_snapshots(&mut self) {
        if !self.wants_workspace_events() {
            // Cheap with no subscriber: one walk above, nothing built. The
            // published record goes stale here, and subscribing syncs it
            // without emitting (see `sync_workspace_snapshots`).
            return;
        }
        let outputs = self.outputs.len();
        for index in 0..outputs {
            let Some((id, output)) = self.outputs.at(index) else {
                continue;
            };
            let Some(active) = self
                .world
                .workspace_window_counts(id, &mut self.workspace_counts)
            else {
                // Unknown to the core: skip, leaving `published` alone for
                // the same reason `refresh_workspaces` skips it. Unreachable
                // after `init_named`/`add_output` (both file `OutputAdded`
                // synchronously before any refresh can run).
                continue;
            };
            let changed = match self
                .workspace_published
                .iter()
                .find(|entry| entry.output == id.0)
            {
                Some(told) => told.active != active || told.counts != self.workspace_counts,
                // Never told: an output added since the subscribe-time sync.
                // Its whole snapshot is new, so it goes out on the tick.
                None => true,
            };
            if !changed {
                // Back where it was sent: a change that reverted before the
                // tick carries nothing.
                if let Some(position) = self
                    .workspace_pending
                    .iter()
                    .position(|entry| entry.output == id.0)
                {
                    self.workspace_pending.remove(position);
                }
                continue;
            }
            match self
                .workspace_pending
                .iter_mut()
                .find(|entry| entry.output == id.0)
            {
                Some(marked) => {
                    // Swapped, not copied: the scratch keeps the slot's old
                    // buffer for the next output, so churn allocates nothing
                    // past the high-water mark. The name is left alone --
                    // connector names are stable for a live output, and a
                    // recreated output is a new id with a new slot below.
                    marked.active = active;
                    std::mem::swap(&mut marked.counts, &mut self.workspace_counts);
                }
                None => {
                    self.workspace_pending.push(WorkspaceSnapshot {
                        output: id.0,
                        name: output.name(),
                        active,
                        counts: std::mem::take(&mut self.workspace_counts),
                    });
                    // The tick may have dropped itself on a quiet screen.
                    self.ensure_ticking();
                }
            }
        }
    }

    /// Records the current snapshots without emitting -- what
    /// `Request::Subscribe` does on the way in when `workspace` is among
    /// the kinds, so the skipped reads while unsubscribed cannot report
    /// the unsubscribed interval as one change later. A fresh subscription
    /// starts silent; read `windows` once for the baseline.
    ///
    /// Outputs with a marked-but-unsent snapshot are left alone: another
    /// subscriber is still waiting on those marks, and the tick carries
    /// them to the newcomer too -- wiping them here would leave that first
    /// subscriber stale with no event to converge on. Only outputs with
    /// nothing marked get their published record brought to the present.
    pub(super) fn sync_workspace_snapshots(&mut self) {
        let outputs = self.outputs.len();
        for index in 0..outputs {
            let Some((id, output)) = self.outputs.at(index) else {
                continue;
            };
            if self
                .workspace_pending
                .iter()
                .any(|entry| entry.output == id.0)
            {
                continue;
            }
            let Some(active) = self
                .world
                .workspace_window_counts(id, &mut self.workspace_counts)
            else {
                continue;
            };
            let counts = std::mem::take(&mut self.workspace_counts);
            match self
                .workspace_published
                .iter_mut()
                .find(|entry| entry.output == id.0)
            {
                Some(told) => {
                    told.active = active;
                    told.name = output.name();
                    told.counts = counts;
                }
                None => self.workspace_published.push(WorkspaceSnapshot {
                    output: id.0,
                    name: output.name(),
                    active,
                    counts,
                }),
            }
        }
    }

    /// Forgets the snapshots of a removed output, so a later refresh never
    /// emits for it. Called from `State::remove_output`, beside the event
    /// that removal already sends: a gone output's occupancy is nothing to
    /// report.
    pub(crate) fn forget_workspace_output(&mut self, id: OutputId) {
        self.workspace_published
            .retain(|entry| entry.output != id.0);
        self.workspace_pending.retain(|entry| entry.output != id.0);
    }

    /// Sends every marked occupancy snapshot to its output's subscribers.
    ///
    /// Called once per frame tick, before the subscriber tails drain, so at
    /// most one event per output goes out per tick however many applies
    /// marked it in between. What went out is published as sent -- the next
    /// refresh diffs against what subscribers now hold. Tests drive this
    /// directly instead of the timer.
    pub fn flush_workspace_events(&mut self) {
        if self.workspace_pending.is_empty() {
            return;
        }
        let mut pending = std::mem::take(&mut self.workspace_pending);
        for snapshot in pending.drain(..) {
            // One encode per changed output, carried to every subscriber by
            // the shared tail -- the same shape as every other emitter.
            self.emit_workspaces(snapshot.clone());
            if let Some(told) = self
                .workspace_published
                .iter_mut()
                .find(|entry| entry.output == snapshot.output)
            {
                told.active = snapshot.active;
                told.name = snapshot.name;
                told.counts = snapshot.counts;
            } else {
                self.workspace_published.push(snapshot);
            }
        }
        // The buffer survives for the next tick's marks.
        self.workspace_pending = pending;
    }

    /// Whether occupancy snapshots are waiting for the tick -- which is
    /// what keeps the frame timer alive until they go out (see
    /// `frame_tick`). A subscriber with nothing marked needs nothing from
    /// the tick; the next mark wakes it.
    pub fn workspace_events_pending(&self) -> bool {
        !self.workspace_pending.is_empty()
    }

    /// Sends an occupancy snapshot to every `Workspace` subscriber.
    /// See the module doc for what happens to one that stops reading.
    ///
    /// Called from the per-tick flush rather than the refresh itself, so
    /// this runs at most once per output per tick -- and only when the
    /// snapshot actually moved. A refresh whose counts came back equal
    /// never reaches this.
    pub fn emit_workspaces(&mut self, event: WorkspaceSnapshot) {
        let Ok(line) = encode(&Response::Workspaces(event)) else {
            return;
        };
        self.emit(EventKind::Workspace, &line);
    }
}
