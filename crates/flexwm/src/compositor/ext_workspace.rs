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
//! `flexwm_core` has real workspaces already ([`World::workspaces`]), and
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
//!   displays. There is no IPC action that takes the same number yet --
//!   `flexwm msg action focus-workspace` only steps `up`/`down` -- see
//!   `ROADMAP.md`'s backlog entry on the next `flexwm-ipc` version bump.
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
//! - **`assign`**: there is one workspace group, because there is one output.
//!
//! ## One group, because there is one output
//!
//! The single group carries the one [`Output`] this compositor creates (see
//! `headless.rs`'s `OUTPUT_ID`). Multi-output support has to make this a
//! group per output -- the protocol is built for it (a group is "a set of
//! outputs", and a bar reads its workspaces per group) -- and that is the
//! same list of sites `OUTPUT_ID`'s own doc enumerates, plus
//! [`Action::FocusWorkspaceIndex`], which today means "of the focused
//! output".
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

use flexwm_core::{Action, Workspaces};
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
use super::headless::OUTPUT_ID;
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
/// told**. It is never read as "what the workspaces are" -- that is
/// [`World::workspaces`], re-read on every refresh -- and the two are brought
/// back together in exactly one place ([`State::refresh_workspaces`]), which
/// is also the only writer. Keeping every manager in lockstep is what makes
/// one shared snapshot correct rather than needing one per client: a manager
/// that binds later is built from `published` *after* a refresh, so it starts
/// out agreeing with everyone else.
#[derive(Debug, Default)]
pub struct ExtWorkspaceState {
    managers: Vec<Manager>,
    published: Workspaces,
    /// Reused across refreshes rather than allocated per refresh: this runs
    /// on every `State::apply`.
    changes: Vec<Change>,
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

    /// The manager owning `handle`, and which workspace position it stands
    /// for.
    ///
    /// This is also what makes a stale handle inert without storing a flag on
    /// it: a handle is live exactly while some registered manager still lists
    /// it. One that was `removed` has been truncated out of that list, one
    /// belonging to a manager that was `stop`ped or whose client went away
    /// has no list to be in, and a destroyed-and-reused object id compares
    /// unequal (a [`Weak`]'s id carries a serial). All three fall out as
    /// `None`, which every request handler treats as "ignore", exactly as the
    /// protocol requires for an inert object.
    fn locate(&mut self, handle: &ExtWorkspaceHandleV1) -> Option<(&mut Manager, usize)> {
        self.managers.iter_mut().find_map(|manager| {
            let index = manager
                .workspaces
                .iter()
                .position(|slot| *slot == *handle)?;
            Some((manager, index))
        })
    }
}

/// One client's bound `ext_workspace_manager_v1` and the objects created for
/// it.
#[derive(Debug)]
struct Manager {
    manager: ExtWorkspaceManagerV1,
    /// The one group. [`Weak`] because a client may destroy the group handle
    /// while keeping the manager -- the workspaces stay, unassigned to any
    /// group, which is a state the protocol names explicitly.
    group: Weak<ExtWorkspaceGroupHandleV1>,
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
    /// staged request flexwm supports is `activate`, and an output has
    /// exactly one active workspace -- so replaying a batch of them in order
    /// ends wherever the last one pointed, which is what this holds. A second
    /// `activate` before a `commit` overwrites the first, at the cost of one
    /// `usize`, so a client cannot make this grow.
    pending_activate: Option<usize>,
}

impl Manager {
    /// Sends one batch of changes and the single `done` that closes it.
    ///
    /// Returns whether this manager can still be kept in step. `false` means
    /// drop it: either its client is gone, or a new object could not be
    /// created for it -- and a manager that misses one `workspace` event can
    /// never be trusted again, because every index after the gap would
    /// address the wrong workspace. Dropping it stops the events; the
    /// client's own objects are untouched and its requests are simply ignored
    /// from then on (see [`ExtWorkspaceState::locate`]).
    fn apply(&mut self, dh: &DisplayHandle, current: Workspaces, changes: &[Change]) -> bool {
        if changes.is_empty() {
            return true;
        }
        let Ok(client) = dh.get_client(self.manager.id()) else {
            return false;
        };
        // Taken once for the whole batch rather than per change: it is the
        // same object every time, and `None` (the client destroyed its group
        // handle) is not a reason to stop sending workspace events.
        let group = self.group.upgrade().ok();
        for change in changes {
            match *change {
                Change::Restated { index, active } => {
                    if let Some(handle) = self.handle(index) {
                        handle.state(workspace_state(active));
                    }
                }
                Change::Removed { index } => {
                    if let Some(handle) = self.handle(index) {
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
                        self.manager.version(),
                        WorkspaceData,
                    );
                    let Ok(handle) = created else {
                        tracing::warn!(
                            index,
                            "could not create an ext_workspace_handle_v1; \
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
                    debug_assert_eq!(
                        index,
                        self.workspaces.len(),
                        "workspace handles are positional"
                    );
                    self.manager.workspace(&handle);
                    describe(&handle, index, active);
                    if let Some(group) = &group {
                        group.workspace_enter(&handle);
                    }
                    self.workspaces.push(handle.downgrade());
                }
            }
        }
        // Whatever was removed above leaves its slot behind; this is what
        // keeps `workspaces.len()` equal to the workspace count, which every
        // index in the next batch is computed against. A no-op when the list
        // grew instead.
        self.workspaces.truncate(current.count);
        self.manager.done();
        true
    }

    /// The live handle at `index`, if the client still has one there.
    fn handle(&self, index: usize) -> Option<ExtWorkspaceHandleV1> {
        self.workspaces.get(index)?.upgrade().ok()
    }
}

impl State {
    /// Brings every bound manager up to date with the core's workspaces.
    ///
    /// Called from `State::apply`, which is the choke point every workspace
    /// change passes through: opening, closing or retitling a window, and
    /// every action (`act`) from a keybinding, from IPC, or from this
    /// protocol's own `activate`. The cost when nothing changed -- which is
    /// most calls, since `apply` also runs for moves within a workspace -- is
    /// one `Option` compare of two `usize`s.
    pub(super) fn refresh_workspaces(&mut self) {
        let Some(current) = self.world.workspaces(OUTPUT_ID) else {
            // No output, so nothing to describe. Unreachable after
            // `headless::init` (which adds the output before the event loop
            // runs) and deliberately *not* published as "zero workspaces":
            // leaving `published` alone means a later refresh diffs against
            // what clients were actually told.
            return;
        };
        if self.ext_workspace.published == current {
            return;
        }
        let dh = self.display_handle.clone();
        let ext = &mut self.ext_workspace;
        // Moved out so the per-manager loop can borrow `ext` mutably, and put
        // back at the end so the allocation survives to the next refresh.
        let mut changes = std::mem::take(&mut ext.changes);
        changes.clear();
        diff::changes(ext.published, current, &mut changes);
        ext.published = current;
        ext.managers
            .retain_mut(|manager| manager.apply(&dh, current, &changes));
        ext.changes = changes;
    }

    /// Applies whatever a client staged before this `commit`.
    ///
    /// Bounds-checked here rather than when `activate` arrived, because the
    /// list can change in between -- that is the whole reason the protocol
    /// batches: a client acts on the state it last saw, and the compositor
    /// decides against the state it has.
    fn commit_workspace_requests(&mut self, manager: &ExtWorkspaceManagerV1) {
        let pending = self
            .ext_workspace
            .managers
            .iter_mut()
            .find(|entry| entry.manager == *manager)
            .and_then(|entry| entry.pending_activate.take());
        let (Some(index), Some(current)) = (pending, self.world.workspaces(OUTPUT_ID)) else {
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
        // Not just an optimisation, and not a behaviour difference either:
        // activating the already-active workspace is a no-op in the core
        // (`focus_workspace_index` sets the index it already has and
        // re-normalises an already-normalised tree). Doing it anyway would
        // mean a client repeating `activate`+`commit` on the workspace it is
        // already on could drive a full `apply` -- an arrange, a configure per
        // window, a render -- as fast as it can write to its socket.
        if index == current.active {
            return;
        }
        self.act(Action::FocusWorkspaceIndex(index));
    }

    /// A client bound a `wl_output`. If it is this compositor's output and
    /// that client has a workspace group, the group has to say so.
    ///
    /// The protocol asks for `output_enter` "whenever an output is assigned
    /// to the workspace group **or a new `wl_output` object is bound by the
    /// client**" -- the second half is this, and without it a bar that binds
    /// the manager before the output (registry order is the server's choice,
    /// not the client's) would see a group with no outputs in it forever.
    pub(super) fn workspace_group_output_bound(&mut self, output: &Output, wl_output: &WlOutput) {
        if self.output.as_ref() != Some(output) {
            return;
        }
        let Some(client) = wl_output.client() else {
            return;
        };
        for manager in &self.ext_workspace.managers {
            // Load-bearing, not tidiness: wayland-backend *panics* when an
            // event carries an object belonging to a different client than
            // the one it is sent to ("Attempting to send an event with
            // objects from wrong client", `rs/server_impl/client.rs`), and a
            // panic here takes every client's session down with it.
            if manager.manager.client().as_ref() != Some(&client) {
                continue;
            }
            let Ok(group) = manager.group.upgrade() else {
                continue;
            };
            group.output_enter(wl_output);
            manager.manager.done();
        }
    }

    /// Builds a freshly bound manager's whole world: the group, its outputs,
    /// a handle per workspace, and the `done` that makes it one atomic
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
        // really was sent.
        self.refresh_workspaces();
        let published = self.ext_workspace.published;
        let created = client.create_resource::<ExtWorkspaceGroupHandleV1, _, State>(
            dh,
            manager.version(),
            GroupData,
        );
        let Ok(group) = created else {
            tracing::warn!("could not create an ext_workspace_group_handle_v1");
            // Still closed with a `done`: a client that waits for one before
            // drawing would otherwise wait forever. `finished` for the same
            // reason as in `Manager::apply`: this manager is never registered,
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
        if let Some(output) = &self.output {
            for wl_output in output.client_outputs(client) {
                group.output_enter(&wl_output);
            }
        }
        let mut workspaces = Vec::with_capacity(published.count);
        for index in 0..published.count {
            let created = client.create_resource::<ExtWorkspaceHandleV1, _, State>(
                dh,
                manager.version(),
                WorkspaceData,
            );
            let Ok(handle) = created else {
                tracing::warn!(index, "could not create an ext_workspace_handle_v1");
                // Deliberately not registered: a manager holding a short list
                // would address every later workspace by the wrong index. So,
                // as above, the incomplete batch is closed and then the object
                // is finished rather than left silent forever.
                manager.done();
                manager.finished();
                return;
            };
            manager.workspace(&handle);
            describe(&handle, index, index == published.active);
            group.workspace_enter(&handle);
            workspaces.push(handle.downgrade());
        }
        manager.done();
        self.ext_workspace.managers.push(Manager {
            manager,
            group: group.downgrade(),
            workspaces,
            pending_activate: None,
        });
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
/// `urgent` is never set: nothing in flexwm marks a window as demanding
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
        state.announce_workspaces(dh, client, manager);
    }
}

impl Dispatch2<ExtWorkspaceManagerV1, State> for ManagerData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
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

    fn destroyed(&self, state: &mut State, _client: ClientId, manager: &ExtWorkspaceManagerV1) {
        // The path an ordinary client disconnect takes, and the only thing
        // that stops a dead client's entry from being walked on every
        // refresh.
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
            // workspace isn't a thing flexwm can do.
            ext_workspace_group_handle_v1::Request::CreateWorkspace { workspace } => {
                tracing::debug!(
                    name = %workspace,
                    "ignoring create_workspace: flexwm creates workspaces itself"
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
                    // Staged, not applied: the protocol's `commit` is what
                    // says the client has finished asking.
                    Some((manager, index)) => manager.pending_activate = Some(index),
                    None => tracing::debug!(
                        "ignoring activate on a workspace handle that is no longer live"
                    ),
                }
            }
            // Never advertised, so never honoured -- see this module's doc.
            ext_workspace_handle_v1::Request::Deactivate
            | ext_workspace_handle_v1::Request::Remove
            | ext_workspace_handle_v1::Request::Assign { .. } => {
                tracing::debug!("ignoring a workspace request flexwm does not advertise");
            }
            ext_workspace_handle_v1::Request::Destroy => {}
            _ => {}
        }
    }
}
