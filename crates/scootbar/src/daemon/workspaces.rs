//! `ext-workspace-v1` on the daemon's side: the events the compositor sends
//! on the connection fd, turned into calls on the workspaces module's
//! shared state (`crate::modules::workspaces`).
//!
//! The module cannot see these itself: they arrive on the Wayland
//! connection's fd, which the loop owns and dispatches centrally, so this
//! glue translates proxies to plain calls here, and the module reports the
//! change through [`crate::modules::Module::on_dispatch`] before the next
//! draw. Nothing here allocates: proxies are compared and stored, names
//! are parsed to numbers at event time.
//!
//! Ignored deliberately: `id` (handles are positions, not identities —
//! scoot sends none), both `capabilities` (the bar only ever sends
//! `activate`+`commit`, which a compositor without the capability ignores),
//! and a `finished` manager's objects afterwards (the shared state stops
//! the module sending anything on them: a request past `finished` is a
//! protocol error, which would kill the bar).

use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, event_created_child};
use wayland_protocols::ext::workspace::v1::client::ext_workspace_group_handle_v1::{
    self, ExtWorkspaceGroupHandleV1,
};
use wayland_protocols::ext::workspace::v1::client::ext_workspace_handle_v1::{
    self, ExtWorkspaceHandleV1, State as WsState,
};
use wayland_protocols::ext::workspace::v1::client::ext_workspace_manager_v1::{
    self, ExtWorkspaceManagerV1,
};

use super::wayland::State;

impl Dispatch<ExtWorkspaceManagerV1, ()> for State {
    event_created_child!(State, ExtWorkspaceManagerV1, [
        ext_workspace_manager_v1::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
        ext_workspace_manager_v1::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ()),
    ]);

    fn event(
        state: &mut Self,
        manager: &ExtWorkspaceManagerV1,
        event: ext_workspace_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let mut shared = state.workspaces.0.borrow_mut();
        // A manager let go (`binds`: no workspaces module placed), or
        // replaced by a later bind, may still have events on the wire. They
        // are not this run's state: a handle it announces is destroyed
        // unseen, and its `finished` must not mark the new manager dead.
        if shared.manager() != Some(manager) {
            match event {
                ext_workspace_manager_v1::Event::WorkspaceGroup { workspace_group } => {
                    workspace_group.destroy();
                }
                ext_workspace_manager_v1::Event::Workspace { workspace } => {
                    workspace.destroy();
                }
                _ => {}
            }
            return;
        }
        match event {
            ext_workspace_manager_v1::Event::WorkspaceGroup { workspace_group } => {
                shared.on_group(workspace_group);
            }
            ext_workspace_manager_v1::Event::Workspace { workspace } => {
                shared.on_workspace(workspace);
            }
            // The batch is whole: commit it, so the loop's next draw shows
            // it.
            ext_workspace_manager_v1::Event::Done => shared.on_done(),
            // Mid-batch or not: the staged half is dropped, the last
            // commit stands, and nothing is sent again.
            ext_workspace_manager_v1::Event::Finished => shared.on_finished(),
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
        let mut shared = state.workspaces.0.borrow_mut();
        match event {
            ext_workspace_group_handle_v1::Event::OutputEnter { output } => {
                shared.on_output_enter(group, output);
            }
            ext_workspace_group_handle_v1::Event::OutputLeave { output } => {
                shared.on_output_leave(group, &output);
            }
            ext_workspace_group_handle_v1::Event::WorkspaceEnter { workspace } => {
                shared.on_workspace_enter(group, &workspace);
            }
            ext_workspace_group_handle_v1::Event::WorkspaceLeave { workspace } => {
                shared.on_workspace_leave(group, &workspace);
            }
            ext_workspace_group_handle_v1::Event::Removed => {
                shared.on_group_removed(group);
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
        let mut shared = state.workspaces.0.borrow_mut();
        match event {
            ext_workspace_handle_v1::Event::Name { name } => {
                shared.on_workspace_name(handle, &name);
            }
            ext_workspace_handle_v1::Event::Coordinates { coordinates } => {
                shared.on_workspace_coordinates(handle, &coordinates);
            }
            ext_workspace_handle_v1::Event::State { state: ws } => {
                let active = matches!(ws, WEnum::Value(bits) if bits.contains(WsState::Active));
                shared.on_workspace_state(handle, active);
            }
            ext_workspace_handle_v1::Event::Removed => {
                shared.on_workspace_removed(handle);
            }
            _ => {}
        }
    }
}
