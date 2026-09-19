//! Window-management actions as they appear on the wire.
//!
//! These mirror `flexwm_core::Action` but are versioned with the protocol, so
//! the core can evolve without breaking clients.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Horizontal {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Vertical {
    Up,
    Down,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    FocusColumn {
        direction: Horizontal,
    },
    FocusWindow {
        direction: Vertical,
    },
    FocusWindowId {
        id: u64,
    },
    MoveColumn {
        direction: Horizontal,
    },
    MoveWindow {
        direction: Vertical,
    },
    ConsumeOrExpel {
        direction: Horizontal,
    },
    CycleColumnWidth,
    FocusWorkspace {
        direction: Vertical,
    },
    /// One specific workspace of the focused output, by its position in the
    /// workspace list (0-based; out of range does nothing) -- the wire half
    /// of `flexwm_core::Action::FocusWorkspaceIndex`, which is what
    /// `ext-workspace-v1`'s `activate` already drives. Stepping
    /// (`FocusWorkspace`) cannot express this: workspaces are renumbered
    /// whenever an empty one is dropped.
    FocusWorkspaceIndex {
        index: usize,
    },
    MoveWindowToWorkspace {
        direction: Vertical,
    },
    CloseFocused,
    Spawn {
        command: Vec<String>,
    },
    Quit,
}
