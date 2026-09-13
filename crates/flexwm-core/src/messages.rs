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
    /// Focus moved for a reason other than a flexwm action: a click, or an app
    /// activating itself. Shells must not report focus changes flexwm itself
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
    FocusWorkspace(Vertical),
    MoveWindowToWorkspace(Vertical),
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
