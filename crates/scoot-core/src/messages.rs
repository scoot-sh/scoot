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
    /// The window asked to enter (`true`) or leave (`false`) fullscreen:
    /// on Wayland, the client's own `xdg_toplevel.set_fullscreen` /
    /// `unset_fullscreen`. Also how a platform reports that a window's
    /// request no longer stands (Wayland discards every toplevel state when
    /// a window unmaps).
    ///
    /// An event rather than an action because it is the window's own doing,
    /// like [`Event::FocusObserved`]: a shell applies it even when it would
    /// refuse the same thing from a user or an agent (scoot does while the
    /// session is locked). What the same intent from a user or an agent looks
    /// like is [`Action::SetFullscreen`]; both land on the same rules, spelled
    /// out on [`Action::ToggleFullscreen`]. Unknown windows are ignored.
    FullscreenRequested {
        id: WindowId,
        fullscreen: bool,
    },
    /// The platform decided, when the window first mapped, that it floats
    /// (`true`) or tiles (`false`): from the window's own properties (a
    /// dialog hint, a parent, a fixed size) or a user's window rule.
    ///
    /// An event rather than an action for the same reason as
    /// [`Event::FullscreenRequested`]: it is about the window, not an intent
    /// from a user or an agent at the keyboard, so a shell applies it even
    /// while it refuses actions (a dialog mapping behind the lock screen is
    /// floating when the session unlocks). The same intent from a user or an
    /// agent is [`Action::SetFloating`]; both land on the rules spelled out
    /// on [`Action::ToggleFloating`].
    ///
    /// `size` is an initial size to ask the window for, in logical pixels
    /// (a window rule's `size`); `None` lets the window choose its own.
    /// Either way it is clamped to the output's usable area, and ignored
    /// when `floating` is `false`. Unknown windows, and a window already in
    /// the asked-for state, are ignored.
    FloatingRequested {
        id: WindowId,
        floating: bool,
        size: Option<Size>,
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
    /// Put the focused window into fullscreen, or take it out. With no window
    /// focused, does nothing.
    ///
    /// What fullscreen means here:
    ///
    /// - **The window covers its output's whole area** -- the gaps and
    ///   whatever the platform reserved at the edges (a bar's exclusive zone)
    ///   included -- whenever its column is the focused one of its output's
    ///   active workspace. [`World::fullscreen_on`](crate::World::fullscreen_on)
    ///   answers "is that happening on this output right now"; every other
    ///   window on that output is then placed invisible.
    /// - **It keeps its column.** In the scrolling strip the column is as
    ///   wide as the whole output while its window is fullscreen, so focusing
    ///   a neighbouring column scrolls to it the ordinary way, and focusing
    ///   back covers the output again. Focused away, the fullscreen window
    ///   keeps its full-output size and is placed exactly where a tiled
    ///   column of that width would be -- one ordinary gap from each
    ///   neighbour, measured within the usable area like every other column
    ///   -- so it may show partly beside the focused window but never
    ///   overlaps it.
    ///   Workspace switching likewise works as it always does.
    /// - **Leaving restores exactly.** The column's width preset is never
    ///   touched, and the scroll offset its workspace had on entry is put
    ///   back on an explicit leave (this action, or the window's own request).
    /// - **At most one per column, and always that column's focused window.**
    ///   The other windows stacked in its column are placed invisible while
    ///   it holds. A window that is not its column's focused window cannot
    ///   enter (the request is ignored), and anything that moves focus to a
    ///   sibling in the column -- a vertical focus step, consuming another
    ///   window into it, focusing a sibling by id -- ends the fullscreen.
    /// - **Moving the window ends it**: consume/expel, and carrying it to
    ///   another workspace or output, each take it out of fullscreen first
    ///   (and do not restore the scroll, which described the layout it just
    ///   left). Moving its column within the strip, or the window within its
    ///   column, does not. An ignored move (at the strip's edge, an unknown
    ///   output) changes nothing, fullscreen included.
    /// - A window that closes while fullscreen simply leaves the layout.
    ToggleFullscreen,
    /// Put one specific window into fullscreen (`true`) or take it out
    /// (`false`), by id: what a taskbar asking on the user's behalf sends.
    /// The same rules as [`Action::ToggleFullscreen`]; an unknown id, or a
    /// window already in the asked-for state, does nothing.
    SetFullscreen {
        id: WindowId,
        fullscreen: bool,
    },
    /// Float the focused window, or put it back in the scrolling strip. With
    /// no window focused, does nothing.
    ///
    /// What floating means here:
    ///
    /// - **Each workspace has a floating layer above its strip**, holding
    ///   windows in stacking order. The most recently focused floating window
    ///   is on top: focusing one (by id, a click reported as
    ///   [`Event::FocusObserved`], or stepping) raises it. A floating window
    ///   belongs to its workspace like a column does, and travels with it.
    /// - **Placement.** A window starts floating centred on its parent (see
    ///   [`WindowInfo::parent`](crate::WindowInfo::parent)) when the parent is
    ///   on the same output's same workspace and visible, otherwise on its
    ///   output's usable area -- decided once, when it starts floating, and
    ///   kept as a centre point: a window that resizes itself grows and
    ///   shrinks around that point rather than jumping. It is always clamped
    ///   inside the usable area (a bar's exclusive zone excluded), shrunk to
    ///   fit it when larger. Carried to another output, it is re-centred
    ///   there.
    /// - **Size.** The window chooses its own (the core asks for nothing --
    ///   [`Placement::requested`](crate::Placement::requested) is `None`),
    ///   unless the platform asked for an initial size
    ///   ([`Event::FloatingRequested`]) or the window drew itself larger than
    ///   the usable area, in which case the core asks for that size, clamped,
    ///   from then on. The rect is the size it last drew
    ///   ([`Event::FrameObserved`]'s `actual`); a window that has not drawn
    ///   yet (and was asked for no size) is placed invisible until it does.
    ///   Frames a floating window draws never teach the strip a minimum.
    /// - **Focus.** A workspace's focus is either in its strip or on its top
    ///   floating window. [`Action::ToggleFloatingFocus`] switches between
    ///   them. With a floating window focused, [`Action::FocusColumn`] leaves
    ///   the floating layer for the strip's focused column (without stepping),
    ///   and [`Action::FocusWindow`] cycles the floating stack -- down raises
    ///   the bottom-most window, up sends the top one to the bottom. The
    ///   strip-only actions ([`Action::MoveColumn`], [`Action::MoveWindow`],
    ///   [`Action::ConsumeOrExpel`], [`Action::CycleColumnWidth`],
    ///   [`Action::SetColumnWidth`]) do nothing. Moving the focused window to
    ///   another workspace or output keeps it floating there, on top and
    ///   focused.
    /// - **Floating takes the window out of its column**; when that empties
    ///   the strip's focused column, strip focus lands on the column to its
    ///   left (for a window that floats as it first maps, that is the column
    ///   that was focused before it opened, and the strip's scroll is put
    ///   back too, so the dialog leaves the strip exactly as it found it).
    ///   **Un-floating inserts it as a new column right of the strip's
    ///   focused column**, at the width it had when it was floated (the
    ///   default width if it was never a column), focused if it was the
    ///   focused window. So floating a focused column and un-floating it
    ///   again puts it back right of its left neighbour: where it was,
    ///   except the leftmost column (it comes back second) and a window
    ///   floated out of a stacked column (it comes back as its own column).
    /// - **Fullscreen.** A floating window can go fullscreen; leaving
    ///   fullscreen puts it back where it floated. Whichever layer a
    ///   fullscreen window is in, the same three rules hold:
    ///   - While it is the focused window it covers its output
    ///     ([`World::fullscreen_on`](crate::World::fullscreen_on)): the rest
    ///     of the workspace is hidden **except its own floating dialogs**
    ///     (windows whose [`parent`](crate::WindowInfo::parent) chain
    ///     reaches it), which stay up, placed above it. Clicking a
    ///     fullscreen app must not hide the dialog it opened: a modal dialog
    ///     blocks its app, and one hidden under it looks like a hang.
    ///   - While a floating window above it has focus (typically that
    ///     dialog), it stays in place, full size, under the floating layer,
    ///     and nothing counts as covering the output. A floating fullscreen
    ///     window then hides the strip and the floating windows below it.
    ///   - While focus is anywhere else it is not shown in front: a column
    ///     stays in its strip slot as usual, a floating one is placed
    ///     invisible.
    ///
    ///   Floating or un-floating a fullscreen window ends its fullscreen
    ///   first.
    /// - A floating window that closes while focused hands focus to its
    ///   parent when the parent is on the same workspace (unless the parent
    ///   is stacked in a column behind a fullscreen sibling: focusing it
    ///   would end that fullscreen), otherwise to the next floating window,
    ///   otherwise to the strip.
    ToggleFloating,
    /// Float one specific window (`true`) or put it back in the strip
    /// (`false`), by id. The same rules as [`Action::ToggleFloating`]; it
    /// does not move focus away from where it is (a window floated while
    /// another floating window has focus goes directly below that one). An
    /// unknown id, or a window already in the asked-for state, does nothing.
    SetFloating {
        id: WindowId,
        floating: bool,
    },
    /// Move focus between the focused workspace's floating layer (its top
    /// window) and its strip (the strip's focused window). Does nothing
    /// when the side focus would move to is empty.
    ToggleFloatingFocus,
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
