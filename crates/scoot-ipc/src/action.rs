//! Window-management actions as they appear on the wire.
//!
//! These mirror `scoot_core::Action` but are versioned with the protocol, so
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
    /// Set the focused column's width to one specific entry of the session's
    /// `column_widths` list, by its position in that list (0-based; out of
    /// range does nothing, like `FocusWorkspaceIndex`) -- the wire half of
    /// `scoot_core::Action::SetColumnWidth`. The absolute half of
    /// `CycleColumnWidth`'s stepping: cycling cannot land on a width, it can
    /// only step past it. Additive: a client that never sends this tag
    /// decodes exactly as before, so no `PROTOCOL_VERSION` bump.
    SetColumnWidth {
        index: usize,
    },
    FocusWorkspace {
        direction: Vertical,
    },
    /// One specific workspace of the focused output, by its position in the
    /// workspace list (0-based; out of range does nothing) -- the wire half
    /// of `scoot_core::Action::FocusWorkspaceIndex`, which is what
    /// `ext-workspace-v1`'s `activate` already drives. Stepping
    /// (`FocusWorkspace`) cannot express this: workspaces are renumbered
    /// whenever an empty one is dropped.
    FocusWorkspaceIndex {
        index: usize,
    },
    MoveWindowToWorkspace {
        direction: Vertical,
    },
    /// Carry the focused window to one specific workspace of the focused
    /// output, by its position in the workspace list (0-based; out of range
    /// leaves the window where it is, like `FocusWorkspaceIndex` leaves
    /// focus where it is) -- the wire half of
    /// `scoot_core::Action::MoveWindowToWorkspaceIndex`. Additive: a client
    /// that never sends this tag decodes exactly as before, so no
    /// `PROTOCOL_VERSION` bump.
    MoveWindowToWorkspaceIndex {
        index: usize,
    },
    /// Carry the focused window to the *active* workspace of another output,
    /// by the output id `scootctl outputs` reports (not a position: output
    /// ids are stable for the session, unlike workspace indices) -- the
    /// wire half of `scoot_core::Action::MoveFocusedWindowToOutput`. An
    /// unknown id leaves the window where it is, like an out-of-range
    /// workspace index. Additive: a client that never sends this tag
    /// decodes exactly as before, so no `PROTOCOL_VERSION` bump.
    MoveFocusedWindowToOutput {
        output: u64,
    },
    /// Move keyboard focus to another output, by the output id `scootctl
    /// outputs` reports -- the wire half of
    /// `scoot_core::Action::FocusOutput`. An unknown id does nothing.
    /// Additive like the move above: no `PROTOCOL_VERSION` bump.
    FocusOutput {
        output: u64,
    },
    /// Put the focused window into fullscreen, or take it out -- the wire
    /// half of `scoot_core::Action::ToggleFullscreen`, whose doc has the
    /// rules (it covers its output edge to edge, bars included, while its
    /// column is focused; leaving restores the layout exactly; moving the
    /// window ends it). With no window focused, does nothing. Additive: a
    /// client that never sends this tag decodes exactly as before, so no
    /// `PROTOCOL_VERSION` bump.
    ToggleFullscreen,
    /// Put one specific window into fullscreen (`fullscreen: true`) or take
    /// it out, by the id `scootctl windows` reports -- the absolute half of
    /// `ToggleFullscreen`, and the wire half of
    /// `scoot_core::Action::SetFullscreen`. Idempotent where the toggle is
    /// not: an agent that wants a window fullscreen need not read its state
    /// first. It does not move focus, so a window in a column that is not
    /// focused is fullscreen but does not cover the output until its column
    /// is focused. An unknown id does nothing, and so does a window stacked
    /// under another in its column (only a column's focused window may be
    /// fullscreen). Additive like the toggle: no `PROTOCOL_VERSION` bump.
    SetFullscreen {
        id: u64,
        fullscreen: bool,
    },
    CloseFocused,
    Spawn {
        command: Vec<String>,
    },
    Quit,
}
