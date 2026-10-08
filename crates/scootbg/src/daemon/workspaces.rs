//! `ext-workspace-v1` on the loop thread: which workspace is active on
//! each output, and the wallpapers that follow.
//!
//! The standard protocol (`crate::workspaces` is the state machine; this
//! is the Wayland glue), bound **only while a mapping exists** (`set
//! --workspace`): never using per-workspace wallpapers binds nothing, so
//! it costs nothing — no extra global, no extra wakeups. Bound late (a
//! mapping made after start-up) or never (a compositor without the
//! protocol, like sway: the mapping is recorded and saved, and applies
//! once a compositor with one is), and released (`stop`) once the last
//! mapping is cleared.
//!
//! No polling: the manager's `done` closes each batch, and only the
//! outputs whose active workspace changed are reconciled. A group that
//! cannot say which output is on which workspace (zero outputs, or more
//! than one) is ignored, so a switch on one output never disturbs
//! another's.

use std::collections::HashMap;
use std::time::Instant;

use wayland_client::backend::ObjectId;
use wayland_client::event_created_child;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::workspace::v1::client::ext_workspace_group_handle_v1::{
    self, ExtWorkspaceGroupHandleV1,
};
use wayland_protocols::ext::workspace::v1::client::ext_workspace_handle_v1::{
    self, ExtWorkspaceHandleV1,
};
use wayland_protocols::ext::workspace::v1::client::ext_workspace_manager_v1::{
    self, ExtWorkspaceManagerV1,
};

use super::change::reconcile;
use super::wayland::{Globals, State};
use crate::choices::Choices;
use crate::print::warn;
use crate::workspaces::Tracker;
use wayland_client::WEnum;

/// The protocol version bound: the only one there is.
const VERSION: u32 = 1;

/// What the daemon keeps for `ext-workspace-v1`.
#[derive(Debug, Default)]
pub struct Workspaces {
    tracker: Tracker,
    /// Bound while a mapping exists, else `None` (zero cost).
    manager: Option<ExtWorkspaceManagerV1>,
    /// `(registry name, version)` while the compositor advertises the
    /// manager and it is not bound yet.
    advertised: Option<(u32, u32)>,
    /// Group objects to tracker keys.
    groups: HashMap<ObjectId, usize>,
    /// Handle objects to (group key, workspace key).
    handles: HashMap<ObjectId, (usize, usize)>,
    /// The last name and active flag announced per handle: re-applied when
    /// a handle enters another group (a move announces no name anew).
    known: HashMap<ObjectId, (Option<String>, bool)>,
    /// Said once: the compositor has no workspace manager.
    warned_missing: bool,
}

impl Workspaces {
    /// Whether registry global `name` is the workspace manager's: for
    /// telling its removal from an output's.
    pub fn is_manager(&self, name: u32) -> bool {
        self.advertised.is_some_and(|(known, _)| known == name)
    }

    /// The compositor advertises the manager global.
    pub fn advertised(&mut self, name: u32, version: u32) {
        self.advertised = Some((name, version));
    }

    /// The manager global went away: forget the advertisement (the live
    /// binding, if any, is dropped by `manager_gone`).
    pub fn unadvertised(&mut self, name: u32) {
        if self.advertised.is_some_and(|(known, _)| known == name) {
            self.advertised = None;
        }
    }
}

/// Binds the manager if mappings exist and it is advertised but unbound;
/// releases it (`stop`) if none exists and it is bound. Idempotent: safe
/// after every change that could flip either side (a `set --workspace`, a
/// `clear --workspace`, a landing trial, a restore, a registry
/// announcement). The steady states cost nothing: unbound, no objects and
/// no events; bound, only the compositor's batches wake the loop.
pub fn ensure_bound(
    workspaces: &mut Workspaces,
    choices: &Choices,
    globals: &Globals,
    qh: &QueueHandle<State>,
) {
    if choices.has_workspace_mappings() {
        if workspaces.manager.is_none() {
            match workspaces.advertised {
                Some((name, version)) => {
                    // Clamped to the protocol's one version, which the
                    // advertisement covers.
                    workspaces.manager =
                        Some(globals.registry.bind::<ExtWorkspaceManagerV1, _, _>(
                            name,
                            version.min(VERSION),
                            qh,
                            (),
                        ));
                }
                None => {
                    if !workspaces.warned_missing {
                        workspaces.warned_missing = true;
                        warn(format_args!(
                            "scootbg: the compositor has no ext-workspace-v1 workspace manager; \
                             workspace wallpapers are recorded and saved, and apply once one does"
                        ));
                    }
                }
            }
        }
    } else if let Some(manager) = workspaces.manager.take() {
        // The last mapping went: stop listening. The tracker's actives no
        // longer switch anything (no mapping reads them), and are dropped
        // with it, so a later mapping starts from the compositor's next
        // full announcement rather than stale names.
        manager.stop();
        *workspaces = Workspaces {
            advertised: workspaces.advertised,
            warned_missing: workspaces.warned_missing,
            ..Default::default()
        };
    }
}

/// The manager global went away: drop the binding and its state, and fall
/// every output back to its own wallpaper.
pub fn manager_gone(state: &mut State, qh: &QueueHandle<State>) {
    if state.workspaces.manager.take().is_none() && state.workspaces.groups.is_empty() {
        return;
    }
    state.workspaces.manager = None;
    state.workspaces.groups.clear();
    state.workspaces.handles.clear();
    state.workspaces.known.clear();
    state.workspaces.tracker = Tracker::default();
    let now = Instant::now();
    let State {
        globals,
        outputs,
        choices,
        images,
        transitions,
        ..
    } = state;
    for entry in outputs.iter_mut() {
        if entry.output.active_workspace().is_some() {
            entry.output.set_active_workspace(None);
            reconcile(
                globals,
                choices,
                &mut images.jobs,
                transitions,
                entry,
                qh,
                now,
            );
        }
    }
}

/// Applies a closed batch: records the outputs whose active workspace
/// changed and reconciles them (with the winning mapping's transition).
fn apply_batch(state: &mut State, qh: &QueueHandle<State>) {
    let changed = state.workspaces.tracker.done();
    if changed.is_empty() {
        return;
    }
    let now = Instant::now();
    let State {
        globals,
        outputs,
        choices,
        images,
        transitions,
        ..
    } = state;
    for (connector, active) in changed {
        let Some(entry) = outputs
            .iter_mut()
            .find(|entry| entry.output.info().name.as_deref() == Some(connector.as_str()))
        else {
            continue;
        };
        if entry.output.active_workspace() == active.as_deref() {
            continue;
        }
        let name = entry.output.info().name.clone();
        let spec = choices.transition_for(name.as_deref(), active.as_deref());
        entry.output.set_active_workspace(active);
        entry.output.request_transition(spec, entry.output.stamp());
        reconcile(
            globals,
            choices,
            &mut images.jobs,
            transitions,
            entry,
            qh,
            now,
        );
    }
}

/// The connector name for `output`, if it is one of the daemon's outputs
/// with a name.
fn connector_of(state: &State, output: &WlOutput) -> Option<String> {
    state.outputs.iter().find_map(|entry| {
        (entry.objects.output.id() == output.id())
            .then(|| entry.output.info().name.clone())
            .flatten()
    })
}

impl Dispatch<ExtWorkspaceManagerV1, ()> for State {
    // The manager's `workspace_group` (opcode 0) and `workspace` (opcode
    // 1) events create their objects: their user data, so dispatch finds
    // them. (Without this the queue panics on the first announcement.)
    event_created_child!(State, ExtWorkspaceManagerV1, [
        0 => (ExtWorkspaceGroupHandleV1, ()),
        1 => (ExtWorkspaceHandleV1, ()),
    ]);

    fn event(
        state: &mut Self,
        _: &ExtWorkspaceManagerV1,
        event: ext_workspace_manager_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_workspace_manager_v1::Event::WorkspaceGroup { workspace_group } => {
                let key = state.workspaces.tracker.group_added();
                state.workspaces.groups.insert(workspace_group.id(), key);
            }
            ext_workspace_manager_v1::Event::Workspace { workspace } => {
                // Unassigned yet: remembered for its group to claim (see
                // `WorkspaceEnter`).
                state
                    .workspaces
                    .known
                    .entry(workspace.id())
                    .or_insert((None, false));
            }
            ext_workspace_manager_v1::Event::Done => apply_batch(state, qh),
            ext_workspace_manager_v1::Event::Finished => {
                // The compositor ends the manager: as if its global went
                // away.
                manager_gone(state, qh);
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        group: &ExtWorkspaceGroupHandleV1,
        event: ext_workspace_group_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(key) = state.workspaces.groups.get(&group.id()).copied() else {
            return;
        };
        match event {
            ext_workspace_group_handle_v1::Event::Capabilities { .. } => {}
            ext_workspace_group_handle_v1::Event::OutputEnter { output } => {
                if let Some(connector) = connector_of(state, &output) {
                    state.workspaces.tracker.output_enter(key, &connector);
                }
            }
            ext_workspace_group_handle_v1::Event::OutputLeave { output } => {
                if let Some(connector) = connector_of(state, &output) {
                    state.workspaces.tracker.output_leave(key, &connector);
                }
            }
            ext_workspace_group_handle_v1::Event::WorkspaceEnter { workspace } => {
                let id = workspace.id();
                let Some(slot) = state.workspaces.tracker.workspace_added(key) else {
                    return;
                };
                state.workspaces.handles.insert(id.clone(), (key, slot));
                // A move announces no name anew: re-apply what is known.
                if let Some((name, active)) = state.workspaces.known.get(&id).cloned() {
                    if let Some(name) = name {
                        state.workspaces.tracker.workspace_named(key, slot, &name);
                    }
                    state.workspaces.tracker.workspace_active(key, slot, active);
                }
            }
            ext_workspace_group_handle_v1::Event::WorkspaceLeave { workspace } => {
                if let Some((group, slot)) = state.workspaces.handles.remove(&workspace.id()) {
                    debug_assert_eq!(group, key);
                    state.workspaces.tracker.workspace_removed(group, slot);
                }
            }
            ext_workspace_group_handle_v1::Event::Removed => {
                state.workspaces.tracker.group_removed(key);
                state.workspaces.groups.remove(&group.id());
                state
                    .workspaces
                    .handles
                    .retain(|_, (group, _)| *group != key);
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtWorkspaceHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ExtWorkspaceHandleV1,
        event: ext_workspace_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = handle.id();
        match event {
            ext_workspace_handle_v1::Event::Id { .. } => {
                // No stable id on scoot, by design (see
                // `crate::workspaces`): names are the key, and this event
                // carries nothing mappable.
            }
            ext_workspace_handle_v1::Event::Name { name } => {
                if let Some(known) = state.workspaces.known.get_mut(&id) {
                    known.0 = Some(name.clone());
                }
                if let Some((group, slot)) = state.workspaces.handles.get(&id).copied() {
                    state.workspaces.tracker.workspace_named(group, slot, &name);
                }
            }
            ext_workspace_handle_v1::Event::Coordinates { .. } => {}
            ext_workspace_handle_v1::Event::State { state: flags } => {
                let active = match flags {
                    WEnum::Value(flags) => flags.contains(ext_workspace_handle_v1::State::Active),
                    // A flag combination the bindings predate: the active
                    // bit is still the active bit.
                    WEnum::Unknown(bits) => bits & 1 != 0,
                };
                if let Some(known) = state.workspaces.known.get_mut(&id) {
                    known.1 = active;
                }
                if let Some((group, slot)) = state.workspaces.handles.get(&id).copied() {
                    state
                        .workspaces
                        .tracker
                        .workspace_active(group, slot, active);
                }
            }
            ext_workspace_handle_v1::Event::Capabilities { .. } => {}
            ext_workspace_handle_v1::Event::Removed => {
                if let Some((group, slot)) = state.workspaces.handles.remove(&id) {
                    state.workspaces.tracker.workspace_removed(group, slot);
                }
                state.workspaces.known.remove(&id);
            }
            _ => {}
        }
    }
}
