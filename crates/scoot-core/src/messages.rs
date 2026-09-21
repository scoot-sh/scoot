//! What flows into and out of [`World`](crate::World).
//!
//! Events are observations a platform shell reports; actions are intents from
//! keybindings or IPC. Neither moves a window directly: placement is read back
//! from [`World::arrange`](crate::World::arrange), and the few imperative
//! leftovers come back as [`Effect`]s.

use crate::geometry::{Rect, Size};
use crate::types::{OutputId, WindowId, WindowInfo};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    OutputAdded {
        id: OutputId,
        area: Rect,
    },
    OutputChanged {
        id: OutputId,
        area: Rect,
    },
    /// The part of an output that ordinary windows may be arranged within,
    /// after whatever the platform reserved at its edges -- on Wayland, the
    /// exclusive zones layer-shell surfaces (bars, docks) asked for.
    ///
    /// In the same coordinate space as [`Event::OutputAdded`]'s `area`, and
    /// intersected with it on the way in, so a stale or oversized rectangle
    /// can only ever describe *less* space than the output has, never more.
    /// Unknown outputs are ignored. A platform that reserves nothing never
    /// needs to send this; one that does should re-send it after changing an
    /// output's geometry, since a reservation measured against the old size
    /// is only re-clamped, not recomputed, by [`Event::OutputChanged`].
    OutputUsableAreaChanged {
        id: OutputId,
        area: Rect,
    },
    /// The output's workspaces move to the focused output; focus stays put.
    OutputRemoved {
        id: OutputId,
    },
    WindowOpened {
        id: WindowId,
        info: WindowInfo,
        /// The output to open on; the focused output when `None` or unknown.
        output: Option<OutputId>,
        /// Whether the window takes focus. A shell listing windows that already
        /// exist, as a macOS adapter does at startup, passes `false`.
        focus: bool,
    },
    WindowChanged {
        id: WindowId,
        info: WindowInfo,
    },
    WindowClosed {
        id: WindowId,
    },
    /// The size a window settled on after being asked for `requested`. Pairing
    /// the two means a late answer to an old request can't be misread. Only a
    /// window ending up larger than asked is informative: it has a minimum the
    /// core didn't know about.
    FrameObserved {
        id: WindowId,
        requested: Size,
        actual: Size,
    },
    /// Focus moved for a reason other than a scoot action: a click, or an app
    /// activating itself. Shells must not report focus changes scoot itself
    /// requested, or a late echo could undo a newer action.
    FocusObserved {
        id: WindowId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Horizontal {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vertical {
    Up,
    Down,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    FocusColumn(Horizontal),
    FocusWindow(Vertical),
    FocusWindowId(WindowId),
    MoveColumn(Horizontal),
    MoveWindow(Vertical),
    /// Join the neighbouring column, or leave the current one if it holds
    /// other windows (niri's consume-or-expel).
    ConsumeOrExpel(Horizontal),
    CycleColumnWidth,
    /// Set the focused column's width to one specific entry of
    /// [`Config::column_widths`](crate::Config::column_widths), by its
    /// position in that list (0-based). The absolute half of
    /// [`Action::CycleColumnWidth`]'s stepping: cycling cannot land on a
    /// width, it can only step past it -- so "widen this column to the whole
    /// output" is one keypress here instead of N cycle steps from wherever
    /// the column already is.
    ///
    /// The same ignore rule as [`Action::FocusWorkspaceIndex`]: an index past
    /// the end of the list does nothing, and with no window focused (or no
    /// output at all) there is no column to resize. Like cycling, choosing a
    /// width on purpose overrides any widths learned from frames.
    SetColumnWidth(usize),
    FocusWorkspace(Vertical),
    /// Activate one specific workspace of the focused output, by its position
    /// in [`World::workspaces`](crate::World::workspaces)' list. Out of range
    /// does nothing.
    ///
    /// Beside [`Action::FocusWorkspace`] rather than replacing it: stepping
    /// and naming a position are different intents, and stepping cannot
    /// express this one -- workspaces are renumbered whenever an empty one is
    /// dropped, so "step down N times" is not "go to workspace N".
    ///
    /// A position, not an identity: nothing in this core gives a workspace a
    /// stable id, and an index only means what it means against the same
    /// [`World::workspaces`](crate::World::workspaces) read it came from.
    FocusWorkspaceIndex(usize),
    MoveWindowToWorkspace(Vertical),
    /// Carry the focused window to one specific workspace of the focused
    /// output, by its position in [`World::workspaces`](crate::World::workspaces)'
    /// list, and follow it there. The index half of [`Action::FocusWorkspaceIndex`]'s
    /// mirror: stepping (`MoveWindowToWorkspace`) cannot express this, for
    /// the same renumbering reason focusing by index exists at all.
    ///
    /// The same out-of-range rule as [`Action::FocusWorkspaceIndex`]: an
    /// index this output doesn't have does nothing, and the window stays
    /// where it is. Moving to the already-active workspace, or with no
    /// window focused, likewise does nothing.
    MoveWindowToWorkspaceIndex(usize),
    /// Carry the focused window to the *active* workspace of another output,
    /// by the output's [`OutputId`](crate::OutputId), and follow it there.
    /// The cross-output half of
    /// [`Action::MoveWindowToWorkspaceIndex`]'s mirror: a workspace index
    /// only means anything within one output's list, so no index can express
    /// "the other screen".
    ///
    /// The same ignore rule as the workspace-index moves: an id this core
    /// doesn't know does nothing, and the window stays where it is. Moving
    /// with no window focused, or to the output the window is already on,
    /// likewise does nothing. The source output keeps whatever neighbour
    /// focus taking the window leaves behind.
    MoveFocusedWindowToOutput(OutputId),
    /// Move keyboard focus to another output, by its
    /// [`OutputId`](crate::OutputId), focusing that output's active
    /// workspace's focused window -- or nothing, when that workspace is
    /// empty, which is what "focused" already means on an output with no
    /// windows. An id this core doesn't know does nothing.
    FocusOutput(OutputId),
    CloseFocused,
    Spawn(Vec<String>),
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Ask the window to close. It stays placed until the platform reports
    /// [`Event::WindowClosed`].
    Close(WindowId),
    Spawn(Vec<String>),
    Quit,
}
