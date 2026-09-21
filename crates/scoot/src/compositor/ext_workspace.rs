//! `ext-workspace-v1`: letting a bar show and switch workspaces.
//!
//! This is the compositor-agnostic successor to the various one-off wlr
//! workspace protocols, and the other half of what an external panel needs
//! alongside `wlr-layer-shell-unstable-v1` (see `layer_shell.rs`): the layer
//! protocol gets a bar onto the screen, this one tells it what to draw there
//! and lets a click on it switch workspaces.
//!
//! Unlike layer shell, the pinned Smithay rev has no helper for this protocol
//! at all (no `ext_workspace` anywhere in it), so the global, the object
//! lifecycle and the event batching are implemented here directly against
//! `wayland-server`. What that hangs off is [`Dispatch2`]/[`GlobalDispatch2`]
//! rather than `wayland_server::Dispatch`: `dispatch.rs` owns a *blanket*
//! `Dispatch`/`GlobalDispatch` impl for [`State`], so a per-interface impl
//! would overlap it (E0119) -- see that module's doc. The user-data types
//! below are the seam that blanket impl forwards through.
//!
//! ## What a workspace is here, and what it is not
//!
//! `scoot_core` has real workspaces already ([`World::workspaces`]), and
//! they are **positions, not identities**: an output holds a `Vec` of them,
//! always ending in exactly one empty workspace, and leaving an emptied
//! workspace drops it, renumbering everything after it. There is no name, no
//! id, and nothing that survives a workspace becoming empty.
//!
//! So the mapping is the honest one: the `n`-th `ext_workspace_handle_v1`
//! this compositor hands a client *is* the `n`-th workspace of the output,
//! for as long as that handle lives. Consequences, all deliberate:
//!
//! - **No `id` event.** The protocol reserves ids for workspaces "likely
//!   stable across multiple sessions"; these are not stable across the next
//!   window closing.
//! - **`name` is the 1-based position** ("1", "2", ...), which is what a bar
//!   displays. The IPC twin is 0-based: `scoot msg action
//!   focus-workspace-index N` drives the same core action `activate` does,
//!   so a bar label `"2"` means `focus-workspace-index 1`, not `2` --
//!   nothing on either side adjusts, and nothing warns. See
//!   `docs/backlog/resolved/protocol-bundle-resolved.md`.
//! - **`coordinates` is that same 1-based position**, as a one-element array
//!   (`"1"` is `[1]`), which the protocol explicitly allows for compositors
//!   that simply number their workspaces -- it requires only that coordinates
//!   be unique within a group, not that they start anywhere in particular.
//!   It is sent because a bar sorting by `name` alone sorts "10" before "2".
//! - **A workspace disappearing is the list getting shorter**, never a hole
//!   in the middle: the tail handles are `removed`, and what was at index 2
//!   is now whatever the core has at index 2. A bar redraws from the state it
//!   is told, so this reads correctly; a client that cached "handle X is my
//!   music workspace" would not, and this protocol's own `id` event is the
//!   thing that would let it, which is exactly why none is sent.
//!
//! ## Capabilities: what is deliberately not implemented
//!
//! Advertised: `activate` on a workspace. Nothing else, because nothing else
//! exists in the core to expose -- and the protocol's own rule is that a
//! compositor ignores requests whose capability it does not advertise:
//!
//! - **`deactivate`**: an output always has exactly one active workspace.
//!   There is no state where none is active to deactivate into.
//! - **`remove` / `create_workspace`**: workspaces are created and dropped by
//!   the layout itself (a new one appears as soon as the trailing empty one
//!   is used, an emptied one is dropped when it is left). A user cannot
//!   create or delete one, so a client cannot either.
//! - **`assign`**: a workspace belongs to the output whose group announced
//!   it. A client cannot move one, because nothing in the compositor can
//!   either -- windows stay on the output they opened on (see below).
//!
//! ## A group per output
//!
//! Every output gets its own group carrying that output (see
//! [`Outputs::iter_with_ids`](super::outputs::Outputs::iter_with_ids)): a
//! bar reads its workspaces per group, which is what the protocol is built
//! for. Each group's handles are positions in *that* output's list, and each
//! output's active index moves independently -- switching on one output
//! never disturbs another's.
//!
//! What `activate` targets is the group its handle came from, not the
//! focused output: the bounds and already-active checks run against that
//! output's list. A real switch still goes through
//! [`Action::FocusWorkspaceIndex`], which is focused-output-relative, so a
//! switch pending on an output that is not the focused one is ignored rather
//! than misrouted (see `commit_workspace_requests`). That branch is
//! unreachable while windows only open on the first output -- every other
//! output's list is permanently the single empty workspace -- and a later
//! phase that moves windows across outputs is where it becomes reachable,
//! alongside an output-targeted action to serve it.
//!
//! ## Batching
//!
//! Every change goes out as: the events, then one `done` on the manager. The
//! whole point of `done` in this protocol family is that a client may see an
//! inconsistent list between events (two workspaces active at once while one
//! is being deactivated and another activated), so the events for one change
//! are computed as a set (`diff.rs`) and flushed with exactly one `done` --
//! one per batch, never one per event, and none at all when the workspaces
//! did not change. (A client that destroyed all of its own handles can still
//! see a bare `done`, since the change it closes was real even though none of
//! its events had anywhere to go.)
//!
//! Requests go the same way in reverse: `activate` is *staged* and applied
//! when the client sends `commit`, which is what the protocol requires ("the
//! compositor must process a series of requests preceding a commit request
//! atomically"). A client that never sends `commit` never switches anything.

use scoot_core::{Action, OutputId, Workspaces};
use smithay::output::Output;
use smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_group_handle_v1::{
    self, ExtWorkspaceGroupHandleV1, GroupCapabilities,
};
use smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_handle_v1::{
    self, ExtWorkspaceHandleV1, State as WorkspaceState, WorkspaceCapabilities,
};
use smithay::reexports::wayland_protocols::ext::workspace::v1::server::ext_workspace_manager_v1::{
    self, ExtWorkspaceManagerV1,
};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::{
    Client, DataInit, DisplayHandle, New, Resource, Weak,
};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use super::State;
use diff::Change;

mod diff;

#[cfg(test)]
mod tests;

/// The only version of this protocol there is. Bumping it means implementing
/// whatever a version 2 adds, not just raising the number: a client binds at
/// the version it understands and expects every event that version defines.
const VERSION: u32 = 1;

/// Everything this compositor keeps for `ext-workspace-v1`.
///
/// The `published` snapshot is the one piece of cached state here, and it
/// means exactly one thing: **what every registered manager has already been
/// told, per output**. It is never read as "what the workspaces are" -- that
/// is [`World::workspaces`], re-read on every refresh -- and the two are
/// brought back together in exactly one place
/// ([`State::refresh_workspaces`]), which is also the only writer. Keeping
/// every manager in lockstep is what makes one shared snapshot correct rather
/// than needing one per client: a manager that binds later is built from
/// `published` *after* a refresh, so it starts out agreeing with everyone
/// else. One entry per output, in creation order.
#[derive(Debug, Default)]
pub struct ExtWorkspaceState {
    managers: Vec<Manager>,
    published: Vec<(OutputId, Workspaces)>,
    /// Reused across refreshes rather than allocated per refresh: this runs
    /// on every `State::apply`.
    changes: Vec<Change>,
    /// Scratch for one refresh's per-output snapshots -- `published`'s
    /// challenger -- reused for the same reason. Its allocation survives
    /// across refreshes; only the previous snapshot's is ever dropped, and
    /// that one becomes this one on the next pass (see `refresh_workspaces`).
    current: Vec<(OutputId, Workspaces)>,
}

impl ExtWorkspaceState {
    /// Creates the `ext_workspace_manager_v1` global.
    ///
    /// The `GlobalId` is dropped: nothing removes this global for the life of
    /// the process, and dropping the id does not remove it either.
    pub fn new(dh: &DisplayHandle) -> Self {
        let _ = dh.create_global::<State, ExtWorkspaceManagerV1, _>(VERSION, ManagerGlobalData);
        Self::default()
    }

    /// The manager owning `handle`, which output's group it stands on, and
    /// which workspace position within that group.
    ///
    /// This is also what makes a stale handle inert without storing a flag on
    /// it: a handle is live exactly while some registered manager still lists
    /// it. One that was `removed` has been truncated out of that list, one
    /// belonging to a manager that was `stop`ped or whose client went away
    /// has no list to be in, and a destroyed-and-reused object id compares
    /// unequal (a [`Weak`]'s id carries a serial). All three fall out as
    /// `None`, which every request handler treats as "ignore", exactly as the
    /// protocol requires for an inert object.
    fn locate(&mut self, handle: &ExtWorkspaceHandleV1) -> Option<(&mut Manager, OutputId, usize)> {
        self.managers.iter_mut().find_map(|manager| {
            manager
                .groups
                .iter()
                .find_map(|group| {
                    group
                        .workspaces
                        .iter()
                        .position(|slot| *slot == *handle)
                        .map(|index| (group.output, index))
                })
                .map(|(output, index)| (manager, output, index))
        })
    }
}

/// One client's bound `ext_workspace_manager_v1` and the objects created for
/// it: one group per output.
#[derive(Debug)]
struct Manager {
    manager: ExtWorkspaceManagerV1,
    groups: Vec<Group>,
}

/// One output's group and its workspace handles, as one client sees them.
#[derive(Debug)]
struct Group {
    /// Which output's workspaces this group announces.
    output: OutputId,
    /// The group object. `None` until the first refresh (or bind) creates it;
    /// a dead [`Weak`] afterwards because the client destroyed its group
    /// handle while keeping the manager -- the workspaces stay, unassigned
    /// to any group, which is a state the protocol names explicitly.
    group: Option<Weak<ExtWorkspaceGroupHandleV1>>,
    /// One slot per workspace, in the core's own order: slot `n` is the
    /// `n`-th workspace of the output. [`Weak`] again, and for a sharper
    /// reason -- a client may `destroy` any of these at any time, and a
    /// destroyed slot must stay in place rather than being compacted out, or
    /// every later index would silently address the wrong workspace.
    workspaces: Vec<Weak<ExtWorkspaceHandleV1>>,
    /// The workspace this client asked to activate since its last `commit`,
    /// if any.
    ///
    /// One slot rather than a queue, and it is not a simplification: the only
    /// staged request scoot supports is `activate`, and an output has
    /// exactly one active workspace -- so replaying a batch of them in order
    /// ends wherever the last one pointed, which is what this holds. A second
    /// `activate` before a `commit` overwrites the first, at the cost of one
    /// `usize`, so a client cannot make this grow.
    pending_activate: Option<usize>,
}

impl Manager {
    /// Sends one output's batch of changes to this manager's group for it.
    ///
    /// Creates the group on first sight -- with the `workspace_group` and
    /// `capabilities` events and the `output_enter`s for the `wl_output`s
    /// this client holds, exactly as a bind-time group is built -- so a
    /// manager bound before an output existed still gets that output's
    /// group once there is something to say about it.
    ///
    /// Returns whether this manager can still be kept in step. `false` means
    /// drop it: either its client is gone, or a new object could not be
    /// created for it -- and a manager that misses one `workspace` event can
    /// never be trusted again, because every index after the gap would
    /// address the wrong workspace. Dropping it stops the events; the
    /// client's own objects are untouched and its requests are simply ignored
    /// from then on (see [`ExtWorkspaceState::locate`]). The `done` that
    /// closes the batch is the caller's: one per manager per refresh, not
    /// one per output.
    fn apply_output(
        &mut self,
        dh: &DisplayHandle,
        client: &Client,
        output: OutputId,
        smithay_output: Option<&Output>,
        current: Workspaces,
        changes: &[Change],
    ) -> bool {
        let version = self.manager.version();
        // The entry, created (but not the object) on first sight.
        let position = match self.groups.iter().position(|group| group.output == output) {
            Some(position) => position,
            None => {
                self.groups.push(Group {
                    output,
                    group: None,
                    workspaces: Vec::new(),
                    pending_activate: None,
                });
                self.groups.len() - 1
            }
        };
        // The object, created on first sight with everything a bind-time
        // group carries. A dead `Weak` (the client destroyed its group
        // handle) is not resurrected: the workspaces below are still sent,
        // unassigned to any group, exactly as for a group destroyed at bind
        // time.
        let group = match &self.groups[position].group {
            None => {
                let created = client
                    .create_resource::<ExtWorkspaceGroupHandleV1, _, State>(dh, version, GroupData);
                let Ok(object) = created else {
                    tracing::warn!(
                        ?output,
                        "could not create an ext_workspace_group_handle_v1; \
                         dropping this workspace manager"
                    );
                    // Closed even though the batch is incomplete, so a
                    // client waiting for `done` before it redraws is left
                    // out of date rather than waiting forever, then
                    // `finished` so it knows nothing more is coming --
                    // dropping the entry below only stops *this* side
                    // tracking it, the client's object would otherwise
                    // dangle with no events for the rest of its life.
                    // `finished` is a destructor event, so nothing may be
                    // sent on the manager after it; the `destroyed`
                    // callback it queues runs later (wayland-backend
                    // defers destructors to its next cleanup, never
                    // re-entrantly through this `retain_mut`) and re-runs
                    // the same idempotent `retain`.
                    self.manager.done();
                    self.manager.finished();
                    return false;
                };
                self.manager.workspace_group(&object);
                // Mandatory once per object even though it is empty here --
                // the protocol requires `capabilities` after creation, and
                // an empty set is how a client learns not to offer a "new
                // workspace" button.
                object.capabilities(GroupCapabilities::empty());
                if let Some(smithay_output) = smithay_output {
                    for wl_output in smithay_output.client_outputs(client) {
                        object.output_enter(&wl_output);
                    }
                }
                self.groups[position].group = Some(object.downgrade());
                Some(object)
            }
            Some(weak) => weak.upgrade().ok(),
        };
        // Taken once for the whole batch rather than per change: it is the
        // same object every time, and `None` (the client destroyed its group
        // handle) is not a reason to stop sending workspace events.
        let group_entry = &mut self.groups[position];
        for change in changes {
            match *change {
                Change::Restated { index, active } => {
                    if let Some(handle) = group_entry.handle(index) {
                        handle.state(workspace_state(active));
                    }
                }
                Change::Removed { index } => {
                    if let Some(handle) = group_entry.handle(index) {
                        // The group has to let go of a workspace before it is
                        // removed: the protocol is explicit that a compositor
                        // "must only remove a workspace not currently
                        // belonging to any workspace_group".
                        if let Some(group) = &group {
                            group.workspace_leave(&handle);
                        }
                        handle.removed();
                    }
                }
                Change::Added { index, active } => {
                    let created = client.create_resource::<ExtWorkspaceHandleV1, _, State>(
                        dh,
                        version,
                        WorkspaceData,
                    );
                    let Ok(handle) = created else {
                        tracing::warn!(
                            index,
                            "could not create an ext_workspace_handle_v1; \
                             dropping this workspace manager"
                        );
                        // Same incomplete-batch close as above.
                        self.manager.done();
                        self.manager.finished();
                        return false;
                    };
                    debug_assert_eq!(
                        index,
                        group_entry.workspaces.len(),
                        "workspace handles are positional"
                    );
                    self.manager.workspace(&handle);
                    describe(&handle, index, active);
                    if let Some(group) = &group {
                        group.workspace_enter(&handle);
                    }
                    group_entry.workspaces.push(handle.downgrade());
                }
            }
        }
        // Whatever was removed above leaves its slot behind; this is what
        // keeps `workspaces.len()` equal to the workspace count, which every
        // index in the next batch is computed against. A no-op when the list
        // grew instead.
        group_entry.workspaces.truncate(current.count);
        true
    }
}

impl Group {
    /// The live handle at `index`, if the client still has one there.
    fn handle(&self, index: usize) -> Option<ExtWorkspaceHandleV1> {
        self.workspaces.get(index)?.upgrade().ok()
    }
}

impl State {
    /// Brings every bound manager up to date with every output's workspaces.
    ///
    /// Called from `State::apply`, which is the choke point every workspace
    /// change passes through: opening, closing or retitling a window, and
    /// every action (`act`) from a keybinding, from IPC, or from this
    /// protocol's own `activate`. The cost when nothing changed -- which is
    /// most calls, since `apply` also runs for moves within a workspace -- is
    /// one snapshot compare per output, with no allocation: the challenger
    /// is built in the reused `current` scratch and both buffers are handed
    /// back afterwards.
    pub(super) fn refresh_workspaces(&mut self) {
        if self.outputs.is_empty() {
            // No output, so nothing to describe. Unreachable after
            // `headless::init` (which adds the output before the event loop
            // runs) and deliberately *not* published as "zero workspaces":
            // leaving `published` alone means a later refresh diffs against
            // what clients were actually told.
            return;
        }
        let dh = self.display_handle.clone();
        let ext = &mut self.ext_workspace;
        // Moved out so the per-manager loop can borrow `ext`, and put back
        // at the end so the allocations survive to the next refresh.
        let mut changes = std::mem::take(&mut ext.changes);
        let mut current = std::mem::take(&mut ext.current);
        current.clear();
        for (id, _) in self.outputs.iter_with_ids() {
            // Unknown to the core: skip, leaving `published` alone for the
            // same reason as the no-output return above. Unreachable after
            // `init_named`/`add_output` (both file `OutputAdded`
            // synchronously before any refresh can run), but a second lookup
            // that could come back empty is exactly what `resize_output`
            // refuses to do twice.
            if let Some(workspaces) = self.world.workspaces(id) {
                current.push((id, workspaces));
            }
        }
        if current.is_empty() {
            // Every output unknown to the core: describe nothing and publish
            // nothing, the same reason as the no-output return above. Without
            // this the swap below would forget what clients were told.
            // Unreachable for the same reason the per-output skip is, but the
            // cost is one `is_empty`.
            ext.changes = changes;
            ext.current = current;
            return;
        }
        // Fast path: every snapshot matches -- no events, and no `done`.
        let unchanged = current.len() == ext.published.len()
            && current
                .iter()
                .zip(ext.published.iter())
                .all(|(a, b)| a == b);
        if !unchanged {
            ext.managers.retain_mut(|manager| {
                let Ok(client) = dh.get_client(manager.manager.id()) else {
                    return false;
                };
                let mut changed = false;
                for (id, state) in &current {
                    let told = ext
                        .published
                        .iter()
                        .find(|(known, _)| known == id)
                        .map(|(_, workspaces)| *workspaces)
                        .unwrap_or_default();
                    if told == *state {
                        continue;
                    }
                    changes.clear();
                    diff::changes(told, *state, &mut changes);
                    changed |= !changes.is_empty();
                    let smithay_output = self.outputs.get(*id);
                    if !manager.apply_output(&dh, &client, *id, smithay_output, *state, &changes) {
                        return false;
                    }
                }
                if changed {
                    manager.manager.done();
                }
                true
            });
            // `current` becomes the new snapshot; the previous one becomes
            // the scratch buffer, so neither allocation is lost.
            std::mem::swap(&mut ext.published, &mut current);
            current.clear();
        }
        ext.changes = changes;
        ext.current = current;
    }

    /// Applies whatever clients staged before this `commit` -- every group
    /// with a pending `activate`, in creation order.
    ///
    /// Bounds-checked here rather than when `activate` arrived, because the
    /// list can change in between -- that is the whole reason the protocol
    /// batches: a client acts on the state it last saw, and the compositor
    /// decides against the state it has. Checked against the *group's*
    /// output's list, never another output's: an `activate` names a position
    /// in the list its handle came from.
    fn commit_workspace_requests(&mut self, manager: &ExtWorkspaceManagerV1) {
        loop {
            let next = self
                .ext_workspace
                .managers
                .iter_mut()
                .find(|entry| entry.manager == *manager)
                .and_then(|entry| {
                    entry.groups.iter_mut().find_map(|group| {
                        group
                            .pending_activate
                            .take()
                            .map(|index| (group.output, index))
                    })
                });
            let Some((output, index)) = next else {
                break;
            };
            self.commit_one_workspace_request(output, index);
        }
    }

    /// Applies one staged `activate` on one output's group.
    fn commit_one_workspace_request(&mut self, output: OutputId, index: usize) {
        let Some(current) = self.world.workspaces(output) else {
            // The output is gone: nothing to switch to. Unreachable --
            // outputs are never removed -- and ignoring is the safe answer
            // either way: a refused request must not disturb anything, so
            // the session comes back as the user left it.
            return;
        };
        if index >= current.count {
            tracing::debug!(
                index,
                count = current.count,
                "ignoring activate: that workspace no longer exists"
            );
            return;
        }
        if self.session_lock.is_locked() {
            tracing::debug!(
                index,
                "ignoring an ext-workspace activate: the session is locked"
            );
            return;
        }
        // A real switch pending on an output that is not the focused one.
        // Unreachable while windows only open on the first output (see
        // `shell.rs`): every other output's list is permanently the single
        // empty workspace, so a valid, non-active index cannot name one. The
        // switch below goes through `FocusWorkspaceIndex`, which is
        // focused-output-relative, so reaching it for another output would
        // switch the wrong screen -- the harm this phase pins. Ignored
        // rather than misrouted, loudly in debug builds.
        if Some(output) != self.world.focused_output() && index != current.active {
            debug_assert!(
                false,
                "workspace switch for a non-focused output has no output-targeted action yet"
            );
            tracing::debug!(
                ?output,
                index,
                "ignoring an ext-workspace activate for a non-focused output"
            );
            return;
        }
        // Mirrors `input.rs`'s `focus_under_pointer`, which clears this on the
        // line before its own `act(FocusWindowId)` -- and what
        // `wlr_toplevel_activate` and `request_activation` do for their own
        // focus requests: without it `layer_shell.rs`'s `layer_keyboard_focus`
        // hands the keyboard straight back to a still-mapped `on_demand`
        // surface named here, so the refresh `act` ends in would re-derive
        // the panel -- which is exactly what sent this request, and stays
        // mapped -- instead of the window. The click is spent only once the
        // request is known to be honored, which is why the stale-index return
        // and the lock gate above come first: a refused request must not
        // disturb anything, so the session comes back as the user left it.
        // `act`'s gate remains as the backstop `shell.rs` describes it as.
        self.clicked_layer = None;
        // Not just an optimisation, and not a behaviour difference either:
        // activating the already-active workspace is a no-op in the core
        // (`focus_workspace_index` sets the index it already has and
        // re-normalises an already-normalised tree). Doing it anyway would
        // mean a client repeating `activate`+`commit` on the workspace it is
        // already on could drive a full `apply` -- an arrange, a configure per
        // window, a render -- as fast as it can write to its socket. So, like
        // `wlr_toplevel_activate`'s already-focused fast path, this skips
        // `act` and runs only the keyboard half -- now with `clicked_layer`
        // already cleared, so it reaches the window rather than stopping at
        // the taskbar. The same request over IPC (`FocusWorkspaceIndex`, PR
        // #53) spends the click unconditionally, and this path agrees with it
        // rather than differing by transport.
        if index == current.active {
            self.refresh_keyboard_focus();
            return;
        }
        self.act(Action::FocusWorkspaceIndex(index));
    }

    /// A client bound a `wl_output`. If it is an output this compositor has
    /// and that client has the workspace group for it, the group has to say
    /// so.
    ///
    /// The protocol asks for `output_enter` "whenever an output is assigned
    /// to the workspace group **or a new `wl_output` object is bound by the
    /// client**" -- the second half is this, and without it a bar that binds
    /// the manager before the output (registry order is the server's choice,
    /// not the client's) would see a group with no outputs in it forever.
    pub(super) fn workspace_group_output_bound(&mut self, output: &Output, wl_output: &WlOutput) {
        // The output's own group, not every group: a bind of one output must
        // not tell another output's group it entered.
        let Some(id) = self.outputs.id_of(output) else {
            return;
        };
        let bound = wl_output.id();
        for manager in &self.ext_workspace.managers {
            // Load-bearing, not tidiness: wayland-backend *panics* when an
            // event carries an object belonging to a different client than
            // the one it is sent to ("Attempting to send an event with
            // objects from wrong client", `rs/server_impl/client.rs`), and
            // a panic here takes every client's session down with it.
            //
            // Compared as ids rather than through `Resource::client`,
            // because this runs once per manager per `wl_output` bind and
            // any client may provoke it: `same_client_as` is a comparison
            // of the two `ObjectId`s' stored client ids, while `client()`
            // takes the backend's state mutex twice and clones an
            // `Arc<dyn ClientData>` to answer the same question. It is
            // also the *exact* question -- the panic above is literally
            // `o.id.client_id != self.id` on the object argument. A manager
            // that has since died is skipped under the system backend and
            // harmlessly kept under the Rust one, where the event is
            // swallowed as `InvalidId` rather than sent (same file's
            // `get_object`, whose `?` the generated `let _ =` eats).
            if !manager.manager.id().same_client_as(&bound) {
                continue;
            }
            let Some(group) = manager
                .groups
                .iter()
                .find(|group| group.output == id)
                .and_then(|group| group.group.as_ref())
                .and_then(|weak| weak.upgrade().ok())
            else {
                continue;
            };
            group.output_enter(wl_output);
            manager.manager.done();
        }
    }

    /// Builds a freshly bound manager's whole world: one group per output
    /// with its workspaces, and the single `done` that makes it one atomic
    /// picture.
    fn announce_workspaces(
        &mut self,
        dh: &DisplayHandle,
        client: &Client,
        manager: ExtWorkspaceManagerV1,
    ) {
        // First, so `published` is what the core says right now. Everything
        // below is built from `published` rather than from the core directly,
        // which is what keeps this new manager in lockstep with the ones
        // already bound: the next refresh diffs from a snapshot this client
        // really was sent. Borrowed: the failure paths below need `self`
        // back for the budget give-back, and they run only after the borrow
        // ends -- and this is bind rate either way, not a hot path.
        self.refresh_workspaces();
        let mut groups = Vec::with_capacity(self.ext_workspace.published.len());
        for slot in 0..self.ext_workspace.published.len() {
            let (id, state) = self.ext_workspace.published[slot];
            let created = client.create_resource::<ExtWorkspaceGroupHandleV1, _, State>(
                dh,
                manager.version(),
                GroupData,
            );
            let Ok(group) = created else {
                tracing::warn!("could not create an ext_workspace_group_handle_v1");
                // Counted at bind but never registered (see below), so the claim
                // is given back: the client is gone, and a leak here would be a
                // counter that only grows.
                self.bind_budget.release_bind(&client.id(), &manager.id());
                // Still closed with a `done`: a client that waits for one before
                // drawing would otherwise wait forever. `finished` for the same
                // reason as in `apply_output`: this manager is never registered,
                // so it would never be sent anything again, and `finished` is the
                // protocol's only way to say so. Destructor event -- nothing may
                // be sent on `manager` after it, hence the immediate return.
                manager.done();
                manager.finished();
                return;
            };
            manager.workspace_group(&group);
            // Mandatory once per object even though it is empty here -- the
            // protocol requires `capabilities` after creation, and an empty set
            // is how a client learns not to offer a "new workspace" button.
            group.capabilities(GroupCapabilities::empty());
            if let Some(output) = self.outputs.get(id) {
                for wl_output in output.client_outputs(client) {
                    group.output_enter(&wl_output);
                }
            }
            let mut workspaces = Vec::with_capacity(state.count);
            for index in 0..state.count {
                let created = client.create_resource::<ExtWorkspaceHandleV1, _, State>(
                    dh,
                    manager.version(),
                    WorkspaceData,
                );
                let Ok(handle) = created else {
                    tracing::warn!(index, "could not create an ext_workspace_handle_v1");
                    // Same give-back as above: counted, never registered.
                    self.bind_budget.release_bind(&client.id(), &manager.id());
                    // Deliberately not registered: a manager holding a short list
                    // would address every later workspace by the wrong index. So,
                    // as above, the incomplete batch is closed and then the object
                    // is finished rather than left silent forever.
                    manager.done();
                    manager.finished();
                    return;
                };
                manager.workspace(&handle);
                describe(&handle, index, index == state.active);
                group.workspace_enter(&handle);
                workspaces.push(handle.downgrade());
            }
            groups.push(Group {
                output: id,
                group: Some(group.downgrade()),
                workspaces,
                pending_activate: None,
            });
        }
        manager.done();
        self.ext_workspace
            .managers
            .push(Manager { manager, groups });
    }
}

/// Sends everything a newly created workspace handle needs: what it is called,
/// where it sits, what may be asked of it, and whether it is active.
fn describe(handle: &ExtWorkspaceHandleV1, index: usize, active: bool) {
    // 1-based, matching how a bar labels workspaces and how a user counts
    // them. No `id` event: see this module's doc. Derived once and used for
    // both events on purpose: `name` and `coordinates` are documented as the
    // same number, and two independent expressions of "1-based index" is how
    // they silently drifted apart once already.
    let position = index.saturating_add(1);
    handle.name(position.to_string());
    handle.coordinates(coordinates(position));
    handle.capabilities(WorkspaceCapabilities::Activate);
    handle.state(workspace_state(active));
}

/// A workspace's 1-based position as this protocol's `coordinates` array: one
/// dimension, native byte order, which is how a Wayland `array` of `uint`s is
/// carried.
fn coordinates(position: usize) -> Vec<u8> {
    // Saturating rather than `as`: the cast is on a number the core owns, so
    // it cannot realistically reach `u32::MAX` workspaces, but a silent wrap
    // would put two workspaces at the same coordinate, which the protocol
    // forbids within a group.
    let position = u32::try_from(position).unwrap_or(u32::MAX);
    position.to_ne_bytes().to_vec()
}

/// The `state` bitfield for a workspace.
///
/// `urgent` is never set: nothing in scoot marks a window as demanding
/// attention yet (there is no `xdg_activation` support). `hidden` is never set
/// either -- every workspace this compositor has, including the trailing empty
/// one, is one a user can switch to, and the protocol defines `hidden` as
/// "clients attempting to visualize the compositor workspace state should not
/// display such workspaces".
fn workspace_state(active: bool) -> WorkspaceState {
    if active {
        WorkspaceState::Active
    } else {
        WorkspaceState::empty()
    }
}

/// User data on the `ext_workspace_manager_v1` global itself.
struct ManagerGlobalData;

/// ...and on a manager a client has bound.
struct ManagerData;

/// User data on an `ext_workspace_group_handle_v1`.
struct GroupData;

/// User data on an `ext_workspace_handle_v1`.
///
/// Deliberately empty: which workspace a handle stands for is its *position*
/// in its manager's list, not a number stored on the object. Storing the
/// index here would be a second copy of the same fact, free to drift from the
/// list the events are computed against.
struct WorkspaceData;

impl GlobalDispatch2<ExtWorkspaceManagerV1, State> for ManagerGlobalData {
    fn bind(
        &self,
        state: &mut State,
        dh: &DisplayHandle,
        client: &Client,
        resource: New<ExtWorkspaceManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let manager = data_init.init(resource, ManagerData);
        if state.bind_budget.refuse_bind(client, &manager.id()) {
            // Over the shared per-client budget (see `bind_budget.rs`):
            // deferred, not sent here -- `finished` is a destructor event,
            // and sending one inside `bind` panics wayland-backend's bind
            // epilogue. Not counted, not registered, so no later refresh
            // walks it.
            state.defer_bind_refusal(super::bind_budget::RefusedBind::Workspace(manager));
            return;
        }
        state.announce_workspaces(dh, client, manager);
    }
}

impl Dispatch2<ExtWorkspaceManagerV1, State> for ManagerData {
    fn request(
        &self,
        state: &mut State,
        client: &Client,
        manager: &ExtWorkspaceManagerV1,
        request: ext_workspace_manager_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            ext_workspace_manager_v1::Request::Commit => {
                state.commit_workspace_requests(manager);
            }
            ext_workspace_manager_v1::Request::Stop => {
                // Released synchronously rather than left to `destroyed`: a
                // stop-and-rebind in one batch must see the freed slot without
                // waiting for post-batch cleanup. Idempotent with the
                // `destroyed` release -- `finished` queues it, and removing an
                // absent id is a no-op.
                state.bind_budget.release_bind(&client.id(), &manager.id());
                // Unregistered first: `finished` is a destructor event, so
                // the object is gone as soon as it is sent (wayland-backend
                // removes it from the client's object map and queues its
                // `destroyed` callback), and nothing may try to send to it
                // afterwards. `destroyed` runs this same `retain` later,
                // which is idempotent.
                state
                    .ext_workspace
                    .managers
                    .retain(|entry| entry.manager != *manager);
                manager.finished();
            }
            // The request enums are `#[non_exhaustive]`; an opcode this
            // version does not define never reaches here (wayland-backend
            // rejects it first), so there is nothing to do but ignore it --
            // and certainly not panic, which is what `unreachable!()` would
            // make of a future protocol version.
            _ => {}
        }
    }

    fn destroyed(&self, state: &mut State, client: ClientId, manager: &ExtWorkspaceManagerV1) {
        // The path an ordinary client disconnect takes, and the only thing
        // that stops a dead client's entry from being walked on every
        // refresh -- and the path its budget claim takes back, including for
        // a bare destroy with no `stop` before it.
        state.bind_budget.release_bind(&client, &manager.id());
        state
            .ext_workspace
            .managers
            .retain(|entry| entry.manager != *manager);
    }
}

impl Dispatch2<ExtWorkspaceGroupHandleV1, State> for GroupData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _group: &ExtWorkspaceGroupHandleV1,
        request: ext_workspace_group_handle_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            // Ignored, as the protocol says a compositor does with a request
            // whose capability it doesn't advertise -- and this one
            // advertises none. See this module's doc for why creating a
            // workspace isn't a thing scoot can do.
            ext_workspace_group_handle_v1::Request::CreateWorkspace { workspace } => {
                tracing::debug!(
                    name = %workspace,
                    "ignoring create_workspace: scoot creates workspaces itself"
                );
            }
            // The object goes away on its own; the `Weak` held for it simply
            // stops upgrading.
            ext_workspace_group_handle_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch2<ExtWorkspaceHandleV1, State> for WorkspaceData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        handle: &ExtWorkspaceHandleV1,
        request: ext_workspace_handle_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            ext_workspace_handle_v1::Request::Activate => {
                match state.ext_workspace.locate(handle) {
                    // Staged on that output's group, not applied: the
                    // protocol's `commit` is what says the client has
                    // finished asking.
                    Some((manager, output, index)) => {
                        if let Some(group) = manager
                            .groups
                            .iter_mut()
                            .find(|group| group.output == output)
                        {
                            group.pending_activate = Some(index);
                        }
                    }
                    None => tracing::debug!(
                        "ignoring activate on a workspace handle that is no longer live"
                    ),
                }
            }
            // Never advertised, so never honoured -- see this module's doc.
            ext_workspace_handle_v1::Request::Deactivate
            | ext_workspace_handle_v1::Request::Remove
            | ext_workspace_handle_v1::Request::Assign { .. } => {
                tracing::debug!("ignoring a workspace request scoot does not advertise");
            }
            ext_workspace_handle_v1::Request::Destroy => {}
            _ => {}
        }
    }
}
