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
    /// Move keyboard focus to another output by its position in the output
    /// list (0-based, creation order: the first screen, the second screen)
    /// -- the wire half of `scoot_core::Action::FocusOutputIndex`. What the
    /// default `Super+comma` / `Super+period` binds send, so they keep
    /// reaching a monitor that was unplugged and plugged back in (which
    /// comes back under a fresh id). Out of range does nothing, like an
    /// unknown id. Additive like the id form: no `PROTOCOL_VERSION` bump.
    FocusOutputIndex {
        index: usize,
    },
    /// Carry the focused window to the *active* workspace of the output at
    /// position `index` in the output list (0-based, creation order), and
    /// follow it there -- the wire half of
    /// `scoot_core::Action::MoveFocusedWindowToOutputIndex`, and what the
    /// default `Super+Shift+comma` / `Super+Shift+period` binds send. Out of
    /// range leaves the window where it is. Additive: no `PROTOCOL_VERSION`
    /// bump.
    MoveFocusedWindowToOutputIndex {
        index: usize,
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
    /// Float the focused window above its workspace's scrolling strip, or
    /// put it back in the strip as a column right of the strip's focused one
    /// -- the wire half of `scoot_core::Action::ToggleFloating`, whose doc
    /// has the rules (a floating window is centred on its parent or output,
    /// sizes itself, and is raised when focused). With no window focused,
    /// does nothing. Additive: a client that never sends this tag decodes
    /// exactly as before, so no `PROTOCOL_VERSION` bump.
    ToggleFloating,
    /// Float one specific window (`floating: true`) or put it back in the
    /// strip, by the id `scootctl windows` reports -- the absolute half of
    /// `ToggleFloating`, and the wire half of
    /// `scoot_core::Action::SetFloating`. Idempotent where the toggle is not,
    /// and it does not move focus. An unknown id does nothing. Additive like
    /// the toggle: no `PROTOCOL_VERSION` bump.
    SetFloating {
        id: u64,
        floating: bool,
    },
    /// Move focus between the focused workspace's floating windows (the top
    /// one) and its strip (the strip's focused window) -- the wire half of
    /// `scoot_core::Action::ToggleFloatingFocus`. Does nothing when the other
    /// side is empty. Additive: no `PROTOCOL_VERSION` bump.
    ToggleFloatingFocus,
    /// Move a floating window so its top-left corner is at `x`, `y`, in the
    /// same global logical coordinates `scootctl windows` reports `rect` in
    /// -- the wire half of `scoot_core::Action::MoveFloating`. Clamped
    /// inside the usable area of the output it lands on (`windows` then
    /// reports where it went); a position over another output moves it to
    /// that output's active workspace. It does not move focus (unless the
    /// window was focused, when focus goes with it to another output). An
    /// unknown id, a tiled window and a fullscreen one do nothing. Additive:
    /// a client that never sends this tag decodes exactly as before, so no
    /// `PROTOCOL_VERSION` bump.
    MoveFloating {
        id: u64,
        x: i32,
        y: i32,
    },
    /// Ask a floating window to take `width` x `height` logical pixels,
    /// keeping its top-left corner -- the wire half of
    /// `scoot_core::Action::ResizeFloating` with the bottom-right edges
    /// moving. Clamped to the window's own minimum and maximum size and to
    /// the room between its top-left corner and the usable area's bottom
    /// right (the room wins over a minimum that does not fit), and to at
    /// least 1; the window draws the new size when it
    /// answers the configure, so `windows` reports it a frame or so later.
    /// An unknown id, a tiled window and a fullscreen one do nothing.
    /// Additive like the move: no `PROTOCOL_VERSION` bump.
    ResizeFloating {
        id: u64,
        width: u32,
        height: u32,
    },
    CloseFocused,
    Spawn {
        command: Vec<String>,
    },
    Quit,
}
