//! Which workspace is active on each output, from `ext-workspace-v1`.
//!
//! The standard protocol scoot already offers (and any compositor with it,
//! not only scoot), so a wallpaper can follow the active workspace. The
//! Wayland objects live in `daemon::workspaces`; this is the state machine
//! they feed, with no Wayland types, so every ordering is a unit test.
//!
//! ## Groups and outputs
//!
//! Workspaces arrive in groups; a group carries outputs (`output_enter`)
//! and workspaces (`workspace_enter`). On scoot each group carries exactly
//! one output, and each output's active workspace moves independently.
//! Other compositors may group outputs differently: a group carrying zero
//! outputs, or more than one, cannot say which output is on which
//! workspace, so it is ignored (the base wallpaper shows there). That is
//! the conservative rule, and it is also why closing over "the" active
//! workspace globally would be wrong on multi-output sessions.
//!
//! ## Batching
//!
//! Events for one change arrive as a set closed by one `done` on the
//! manager. Until `done`, nothing is reported: between the events a client
//! may see an inconsistent list (two workspaces active at once), and the
//! daemon must never switch a wallpaper on half a batch. `done` returns
//! the outputs whose active workspace changed since the last batch: the
//! daemon records those (`Choices::set_active`) and reconciles them.
//!
//! ## Names, not identities
//!
//! Workspaces are keyed by name, the string the compositor announces. scoot
//! sends no stable id (its workspaces are positions, renumbered as emptied
//! ones drop), so a mapping for "3" follows position 3 across renumbering,
//! and a destroyed workspace's mapping simply waits for a workspace of
//! that name to be active again. A workspace that never announces a name
//! (its `name` event never came) cannot be mapped and never becomes
//! active here.
//!
//! No polling: everything here runs off the Wayland events the daemon's
//! loop already wakes for.

use std::collections::HashMap;

#[cfg(test)]
mod tests;

/// One workspace group, as announced.
#[derive(Debug, Default)]
struct Group {
    /// Connectors entered, in order; removals swap out.
    outputs: Vec<String>,
    /// Handles announced, by stable key (never reused within the group,
    /// for the same reason as the tracker's).
    workspaces: HashMap<usize, Workspace>,
    next_workspace: usize,
    /// The group itself was removed: reported until the next batch drops
    /// it.
    removed: bool,
}

/// One workspace handle, as announced.
#[derive(Debug, Default)]
struct Workspace {
    name: Option<String>,
    active: bool,
    removed: bool,
}

/// Which workspace is active where, from one compositor's announcements.
#[derive(Debug, Default)]
pub struct Tracker {
    /// By stable key (never reused: groups come and go across batches,
    /// and the Wayland glue holds keys across them).
    groups: HashMap<usize, Group>,
    next_group: usize,
    /// What `done` last reported, per connector: what the daemon was told.
    reported: HashMap<String, Option<String>>,
}

impl Tracker {
    /// A group handle was announced: room for it. Returns its key.
    pub fn group_added(&mut self) -> usize {
        let key = self.next_group;
        self.next_group += 1;
        self.groups.insert(key, Group::default());
        key
    }

    /// The group handle at `key` was removed: its connectors report away
    /// at the next batch, and then it is dropped.
    pub fn group_removed(&mut self, key: usize) {
        if let Some(group) = self.groups.get_mut(&key) {
            group.removed = true;
        }
    }

    /// The group at `key` carries the output named `connector`.
    pub fn output_enter(&mut self, key: usize, connector: &str) {
        if let Some(group) = self.groups.get_mut(&key) {
            if !group.outputs.iter().any(|name| name == connector) {
                group.outputs.push(connector.to_owned());
            }
        }
    }

    /// The group at `key` no longer carries `connector`.
    pub fn output_leave(&mut self, key: usize, connector: &str) {
        if let Some(group) = self.groups.get_mut(&key) {
            if let Some(index) = group.outputs.iter().position(|name| name == connector) {
                group.outputs.swap_remove(index);
            }
        }
    }

    /// A workspace handle was announced on the group at `key`: room for
    /// it. Returns its key within the group (`None` for an unknown
    /// group). Keys are stable: later batches address the same handle by
    /// the same key.
    pub fn workspace_added(&mut self, key: usize) -> Option<usize> {
        self.groups.get_mut(&key).map(|group| {
            let workspace = group.next_workspace;
            group.next_workspace += 1;
            group.workspaces.insert(workspace, Workspace::default());
            workspace
        })
    }

    /// The workspace's name.
    pub fn workspace_named(&mut self, key: usize, workspace: usize, name: &str) {
        if let Some(slot) = self
            .groups
            .get_mut(&key)
            .and_then(|group| group.workspaces.get_mut(&workspace))
        {
            slot.name = Some(name.to_owned());
        }
    }

    /// Whether the workspace is active now.
    pub fn workspace_active(&mut self, key: usize, workspace: usize, active: bool) {
        if let Some(slot) = self
            .groups
            .get_mut(&key)
            .and_then(|group| group.workspaces.get_mut(&workspace))
        {
            slot.active = active;
        }
    }

    /// The workspace handle at `workspace` was removed.
    pub fn workspace_removed(&mut self, key: usize, workspace: usize) {
        if let Some(slot) = self
            .groups
            .get_mut(&key)
            .and_then(|group| group.workspaces.get_mut(&workspace))
        {
            slot.removed = true;
        }
    }

    /// The batch closed: the (connector, active workspace) pairs that
    /// changed since the last batch, connectors sorted. `None` is "no
    /// workspace known active" (group gone, ambiguous, or nothing active):
    /// the base wallpaper shows. Removed groups are dropped here, after
    /// their connectors were reported away.
    pub fn done(&mut self) -> Vec<(String, Option<String>)> {
        let mut now: HashMap<&str, Option<&str>> = HashMap::new();
        for group in self.groups.values() {
            if group.removed {
                continue;
            }
            // Exactly one output: attributable. Anything else is ignored
            // (see the module docs).
            let [output] = group.outputs.as_slice() else {
                continue;
            };
            // Exactly one live group per connector: a second group
            // carrying the same output makes it ambiguous, so neither
            // reports. (A HashMap insert would let the last win; the
            // protocol forbids an output in two groups, so any duplicate
            // is a compositor bug worth silence rather than a guess.)
            let output = output.as_str();
            if now.contains_key(output) {
                now.insert(output, None);
                continue;
            }
            let active = group
                .workspaces
                .values()
                .filter(|workspace| !workspace.removed)
                .filter(|workspace| workspace.active)
                .filter_map(|workspace| workspace.name.as_deref())
                .next();
            // Two active at once (a half-applied batch the compositor
            // closed early): the first wins rather than flapping, and the
            // next batch repairs it.
            now.insert(output, active);
        }
        // Connectors told before that no group claims now: away — once.
        // (Staying away reports nothing further; the entry is kept so a
        // group reclaiming the connector is still a change.)
        let mut changed: Vec<(String, Option<String>)> = Vec::new();
        for connector in self.reported.keys() {
            if !now.contains_key(connector.as_str())
                && self
                    .reported
                    .get(connector)
                    .is_some_and(|told| told.is_some())
            {
                changed.push((connector.clone(), None));
            }
        }
        for (connector, active) in &now {
            let active = active.map(str::to_owned);
            if self.reported.get(*connector) != Some(&active) {
                changed.push(((*connector).to_owned(), active));
            }
        }
        changed.sort();
        for (connector, active) in &changed {
            self.reported.insert(connector.clone(), active.clone());
        }
        self.groups.retain(|_, group| !group.removed);
        // Handles removed within a live group go here too: the next batch
        // re-announces nothing for them, and the slots would only grow.
        for group in self.groups.values_mut() {
            group.workspaces.retain(|_, workspace| !workspace.removed);
        }
        changed
    }
}
