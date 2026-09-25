//! The layout tree. Outputs hold workspaces, workspaces hold a scrolling strip
//! of columns, and columns hold a stack of windows. Each operation here changes
//! one node and its children; [`World`](super::World) coordinates across nodes.

use crate::geometry::{Point, Rect, Size};
use crate::messages::{Horizontal, Vertical};
use crate::types::{OutputId, WindowId, WindowInfo};

#[derive(Debug)]
pub(super) struct WindowState {
    pub(super) info: WindowInfo,
    /// Minimum size learned from frames the window refused to go below.
    pub(super) learned_min: Size,
    /// `Some` while the window is fullscreen; see [`Fullscreen`].
    pub(super) fullscreen: Option<Fullscreen>,
    /// `Some` while the window floats; see [`Floating`]. Exactly the windows
    /// in some workspace's floating layer have it (or, with no output yet,
    /// windows waiting in `World::unplaced` that will join one).
    pub(super) floating: Option<Floating>,
    /// The size the window last drew at, from [`Event::FrameObserved`](crate::Event::FrameObserved)'s
    /// `actual`; zero until it has drawn. What a floating window is placed
    /// at -- and, for a tiled window being floated, the size it keeps until
    /// it draws at the one it chooses.
    pub(super) drawn: Size,
}

/// What a floating window carries. Its place in the stacking order is its
/// index in its workspace's `Workspace::floating`; this is the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Floating {
    /// Where the window is centred, relative to its output's `area` origin
    /// (not the usable area's, so a bar appearing or going does not move
    /// it). `None` centres it on the output's usable area, recomputed on
    /// every arrangement -- a window that floated with no parent to centre
    /// on, or one carried to another output.
    pub(super) centre: Option<Point>,
    /// The size the core asks the window for, before clamping to the usable
    /// area: `None` lets the window choose (the protocol's 0x0). Set by an
    /// initial size a platform asked for, and by a window drawing itself
    /// larger than the usable area (so asking it to fit sticks, rather than
    /// being dropped and re-asked on every frame).
    pub(super) request: Option<Size>,
    /// The column width preset the window had when it was floated out of
    /// the strip, so un-floating it restores its width; `None` for a window
    /// that floated as it opened. Clamped into the width list when used,
    /// since a reload may have shortened it.
    pub(super) preset: Option<usize>,
}

/// What a fullscreen window remembers so leaving fullscreen can put the
/// layout back exactly as it was.
///
/// Its column keeps its place in the strip and its width preset untouched
/// the whole time, so the only thing entering fullscreen changes that
/// leaving it has to undo is the scroll: covering the output lines the view
/// up with the column's left edge, and without this the column would come
/// back wherever that left it rather than where it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Fullscreen {
    /// The `view_x` of the window's workspace just before it went
    /// fullscreen. Restored on an explicit leave only (a toggle, or the
    /// window's own request); a leave forced by the layout -- the window
    /// moving to another workspace, consume or expel, focus moving to a
    /// sibling in its column -- drops it, since the layout it described no
    /// longer exists.
    pub(super) view_x: i32,
}

impl WindowState {
    pub(super) fn new(info: WindowInfo) -> Self {
        Self {
            info,
            learned_min: Size::default(),
            fullscreen: None,
            floating: None,
            drawn: Size::default(),
        }
    }

    /// The larger of what the window asks for and what it has been seen to
    /// insist on.
    pub(super) fn min(&self) -> Size {
        let hinted = self.info.hints.min;
        Size::new(
            hinted.w.max(self.learned_min.w),
            hinted.h.max(self.learned_min.h),
        )
    }
}

#[derive(Debug)]
pub(super) struct Column {
    pub(super) windows: Vec<WindowId>,
    pub(super) focused: usize,
    /// Index into [`Config::column_widths`](crate::Config::column_widths).
    pub(super) preset: usize,
}

impl Column {
    fn new(id: WindowId, preset: usize) -> Self {
        Self {
            windows: vec![id],
            focused: 0,
            preset,
        }
    }

    /// Removes the window at `index`, keeping focus on a neighbour.
    fn remove(&mut self, index: usize) -> WindowId {
        let id = self.windows.remove(index);
        if self.focused > index || self.focused >= self.windows.len() {
            self.focused = self.focused.saturating_sub(1);
        }
        id
    }
}

/// Where a window sits inside one workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Slot {
    /// In the strip: column `column`, `index` down its stack.
    Tiled { column: usize, index: usize },
    /// In the floating layer, `index` up from the bottom of the stack.
    Floating { index: usize },
}

#[derive(Debug, Default)]
pub(super) struct Workspace {
    pub(super) columns: Vec<Column>,
    pub(super) focused: usize,
    /// Scroll offset into the strip of columns. It lives beside the columns
    /// rather than inside them because layout takes it as an input.
    pub(super) view_x: i32,
    /// The floating layer, bottom of the stack first: the last entry is on
    /// top, and is the most recently focused floating window.
    pub(super) floating: Vec<WindowId>,
    /// Whether the workspace's focus is on its floating layer (its top
    /// window) rather than its strip. Never `true` with the layer empty --
    /// every removal from it resets this -- and read through
    /// [`Workspace::floating_has_focus`], which also hands focus to the
    /// floating layer of a workspace with no columns.
    pub(super) floating_focused: bool,
}

impl Workspace {
    /// No windows at all, tiled or floating: what decides whether the
    /// workspace survives `Output::normalize`.
    pub(super) fn is_empty(&self) -> bool {
        self.columns.is_empty() && self.floating.is_empty()
    }

    pub(super) fn focused_column(&self) -> Option<&Column> {
        self.columns.get(self.focused)
    }

    /// Whether the workspace's focus is on its floating layer: asked for,
    /// or the only place a window is.
    pub(super) fn floating_has_focus(&self) -> bool {
        !self.floating.is_empty() && (self.floating_focused || self.columns.is_empty())
    }

    /// The window the workspace would focus: its top floating window while
    /// the floating layer has focus, otherwise its strip's focused window.
    pub(super) fn focused_window(&self) -> Option<WindowId> {
        if self.floating_has_focus() {
            return self.floating.last().copied();
        }
        self.strip_focused_window()
    }

    /// The strip's focused window, whatever the floating layer is doing.
    pub(super) fn strip_focused_window(&self) -> Option<WindowId> {
        let column = self.focused_column()?;
        column.windows.get(column.focused).copied()
    }

    /// Where `id` is on this workspace.
    pub(super) fn slot_of(&self, id: WindowId) -> Option<Slot> {
        if let Some(index) = self.floating.iter().position(|&w| w == id) {
            return Some(Slot::Floating { index });
        }
        self.columns.iter().enumerate().find_map(|(c, column)| {
            column
                .windows
                .iter()
                .position(|&w| w == id)
                .map(|i| Slot::Tiled {
                    column: c,
                    index: i,
                })
        })
    }

    fn into_windows(self) -> impl Iterator<Item = WindowId> {
        self.columns
            .into_iter()
            .flat_map(|c| c.windows)
            .chain(self.floating)
    }

    /// Opens `id` as a new column right of focus. It takes the workspace's
    /// focus when asked (off the floating layer too). Otherwise the
    /// workspace's focus stays exactly where it was: a column going into an
    /// empty strip becomes the strip's focused column (it is the only one),
    /// but a floating window that had focus by default -- the strip was
    /// empty, so `floating_has_focus` was true whatever the flag said --
    /// keeps it, which is why the flag is set rather than left to mean
    /// "the strip".
    pub(super) fn insert_column(&mut self, id: WindowId, preset: usize, focus: bool) {
        let floating_had_focus = self.floating_has_focus();
        let at = if self.columns.is_empty() {
            0
        } else {
            self.focused + 1
        };
        self.columns.insert(at, Column::new(id, preset));
        if focus || self.columns.len() == 1 {
            self.focused = at;
        }
        self.floating_focused = !focus && floating_had_focus;
    }

    /// Removes a window, dropping its column if that leaves it empty, and moves
    /// focus to a neighbour.
    pub(super) fn take(&mut self, column: usize, index: usize) -> WindowId {
        let id = self.columns[column].remove(index);
        if self.columns[column].windows.is_empty() {
            self.columns.remove(column);
            if self.focused > column || self.focused >= self.columns.len() {
                self.focused = self.focused.saturating_sub(1);
            }
        }
        id
    }

    /// [`Workspace::take`] for a window leaving the strip for the floating
    /// layer: when that empties the focused column, strip focus lands on the
    /// column to its *left* rather than the right. A window that floats as it
    /// first maps was inserted right of the focused column
    /// ([`Workspace::insert_column`]), so this is the column that had focus
    /// before it opened; and un-floating inserts right of the focused
    /// column, so floating a focused column and un-floating it again puts it
    /// back right of its left neighbour -- where it was, except for the
    /// leftmost column, which comes back second (there is no left neighbour
    /// to land on), and a window floated out of a stacked column, which
    /// comes back as a column of its own.
    pub(super) fn take_for_float(&mut self, column: usize, index: usize) -> WindowId {
        let emptied_focused = column == self.focused && self.columns[column].windows.len() == 1;
        let id = self.take(column, index);
        if emptied_focused && column > 0 {
            self.focused = column - 1;
        }
        id
    }

    /// Puts `id` in the floating layer. On top and focused when `focus`;
    /// otherwise on top only while the strip has focus -- with a floating
    /// window focused it goes directly below it, so the focused window stays
    /// on top and keeps focus.
    pub(super) fn push_floating(&mut self, id: WindowId, focus: bool) {
        if focus || !self.floating_has_focus() {
            self.floating.push(id);
        } else {
            let below_top = self.floating.len() - 1;
            self.floating.insert(below_top, id);
        }
        if focus {
            self.floating_focused = true;
        }
    }

    /// Takes the floating window at `index` out of the floating layer. The
    /// layer's focus flag goes with the last window; otherwise focus stays
    /// on whichever window is now on top.
    pub(super) fn take_floating(&mut self, index: usize) -> WindowId {
        let id = self.floating.remove(index);
        if self.floating.is_empty() {
            self.floating_focused = false;
        }
        id
    }

    /// Raises the floating window at `index` to the top and gives the
    /// floating layer focus.
    pub(super) fn focus_floating(&mut self, index: usize) {
        let id = self.floating.remove(index);
        self.floating.push(id);
        self.floating_focused = true;
    }

    /// Steps the floating stack: `Down` raises the bottom-most window,
    /// `Up` sends the top one to the bottom -- so repeating either visits
    /// every floating window, in opposite orders. A no-op with fewer than two.
    pub(super) fn cycle_floating(&mut self, dir: Vertical) {
        if self.floating.len() < 2 {
            return;
        }
        match dir {
            Vertical::Down => self.floating.rotate_left(1),
            Vertical::Up => self.floating.rotate_right(1),
        }
    }

    pub(super) fn focus(&mut self, column: usize, index: usize) {
        self.focused = column;
        self.columns[column].focused = index;
        self.floating_focused = false;
    }

    pub(super) fn focus_column(&mut self, dir: Horizontal) {
        self.focused = step(self.focused, self.columns.len(), dir == Horizontal::Right);
    }

    pub(super) fn move_column(&mut self, dir: Horizontal) {
        if self.columns.is_empty() {
            return;
        }
        let to = step(self.focused, self.columns.len(), dir == Horizontal::Right);
        self.columns.swap(self.focused, to);
        self.focused = to;
    }

    pub(super) fn focus_window(&mut self, dir: Vertical) {
        if let Some(column) = self.columns.get_mut(self.focused) {
            column.focused = step(column.focused, column.windows.len(), dir == Vertical::Down);
        }
    }

    pub(super) fn move_window(&mut self, dir: Vertical) {
        if let Some(column) = self.columns.get_mut(self.focused) {
            let to = step(column.focused, column.windows.len(), dir == Vertical::Down);
            column.windows.swap(column.focused, to);
            column.focused = to;
        }
    }

    pub(super) fn cycle_preset(&mut self, presets: usize) {
        if let Some(column) = self.columns.get_mut(self.focused) {
            column.preset = (column.preset + 1) % presets.max(1);
        }
    }

    /// Sets the focused column's width preset to `index` -- the absolute
    /// half of [`cycle_preset`](Self::cycle_preset)'s stepping.
    ///
    /// Ignoring rather than clamping, like
    /// [`focus_workspace_index`](Self::focus_workspace_index): a stale or
    /// wild index must not silently land the column on some other width.
    /// An empty workspace has no column to resize. `presets` is the width
    /// list's length, so a stored preset always stays a valid index into it
    /// (any `index`, including every `usize` a wire client can send, past a
    /// zero-length list is ignored too).
    pub(super) fn set_preset(&mut self, index: usize, presets: usize) {
        let Some(column) = self.columns.get_mut(self.focused) else {
            return;
        };
        if index >= presets {
            return;
        }
        column.preset = index;
    }

    /// Joins the neighbouring column, or leaves the current one when it holds
    /// other windows (niri's consume-or-expel). Returns whether the focused
    /// window actually moved -- false at the strip's edge, or on an empty
    /// workspace.
    pub(super) fn consume_or_expel(&mut self, dir: Horizontal) -> bool {
        let c = self.focused;
        let Some(column) = self.columns.get_mut(c) else {
            return false;
        };
        if column.windows.len() > 1 {
            let preset = column.preset;
            let id = column.remove(column.focused);
            let at = match dir {
                Horizontal::Left => c,
                Horizontal::Right => c + 1,
            };
            self.columns.insert(at, Column::new(id, preset));
            self.focused = at;
        } else {
            let target = match dir {
                Horizontal::Left if c > 0 => c - 1,
                Horizontal::Right if c + 1 < self.columns.len() => c + 1,
                _ => return false,
            };
            let id = self.columns.remove(c).windows[0];
            let target = if target > c { target - 1 } else { target };
            let dest = &mut self.columns[target];
            dest.windows.push(id);
            dest.focused = dest.windows.len() - 1;
            self.focused = target;
        }
        true
    }
}

#[derive(Debug)]
pub(super) struct Output {
    pub(super) id: OutputId,
    /// The whole output, in global coordinates. This is what
    /// [`World::outputs`](super::World::outputs) reports and what a platform
    /// describes as "the screen" -- a bar reserving part of it doesn't change
    /// this, only [`Output::usable`]. Nothing places a window from it
    /// directly.
    pub(super) area: Rect,
    /// The part of [`Output::area`] ordinary windows are arranged within:
    /// the whole output, minus whatever the platform reported as reserved at
    /// its edges (on Wayland, layer-shell exclusive zones -- a bar's height).
    ///
    /// **This, not `area`, is what every layout read uses**
    /// ([`World::arrange`](super::World::arrange)'s `place_workspace`,
    /// `fix_view` and `learn_from_frame`); `area` is reported outward and
    /// never laid out within. Kept as a sub-rectangle of `area` by
    /// construction: both write sites
    /// ([`Event::OutputUsableAreaChanged`](crate::Event::OutputUsableAreaChanged)
    /// and an area change) intersect with `area` first, so it can never
    /// describe space the output doesn't have -- including after a resize
    /// makes the output smaller than the reservation was measured against.
    ///
    /// Equal to `area` until a platform says otherwise, which is what makes
    /// a platform that never reports one (the macOS adapter, or Wayland with
    /// no layer-shell client) behave exactly as it did before this existed.
    pub(super) usable: Rect,
    /// Always ends in one empty workspace, as in niri.
    pub(super) workspaces: Vec<Workspace>,
    pub(super) active: usize,
}

impl Output {
    pub(super) fn new(id: OutputId, area: Rect) -> Self {
        Self {
            id,
            area,
            usable: area,
            workspaces: vec![Workspace::default()],
            active: 0,
        }
    }

    /// Moves or resizes the output, keeping whatever the platform reserved
    /// that still fits.
    ///
    /// Re-clamping rather than resetting `usable` to the new `area` is the
    /// conservative direction: a reservation that still fits (a top bar on an
    /// output that grew or shrank) survives, and one that no longer does
    /// shrinks instead of silently handing windows space a bar is still
    /// drawing over. A platform that tracks reservations is expected to
    /// re-report its usable area after changing an output's geometry anyway --
    /// this only decides what holds in between.
    pub(super) fn set_area(&mut self, area: Rect) {
        self.usable = clamp_usable(self.usable, area);
        self.area = area;
    }

    /// Reports what the platform reserved, as the sub-rectangle left over.
    /// Returns whether it changed -- callers re-scroll only when it did, so a
    /// bar repeating the same zone on every one of its own frames costs
    /// nothing.
    pub(super) fn set_usable(&mut self, usable: Rect) -> bool {
        let clamped = clamp_usable(usable, self.area);
        let changed = clamped != self.usable;
        self.usable = clamped;
        changed
    }

    pub(super) fn active_workspace(&self) -> &Workspace {
        &self.workspaces[self.active]
    }

    pub(super) fn active_workspace_mut(&mut self) -> &mut Workspace {
        &mut self.workspaces[self.active]
    }

    pub(super) fn into_windows(self) -> impl Iterator<Item = WindowId> {
        self.workspaces
            .into_iter()
            .flat_map(Workspace::into_windows)
    }

    pub(super) fn focus_workspace(&mut self, dir: Vertical) {
        self.active = step(self.active, self.workspaces.len(), dir == Vertical::Down);
        self.normalize();
    }

    /// Activates the workspace at `index`, ignoring one this output doesn't
    /// have.
    ///
    /// Ignoring rather than clamping: an index past the end is a caller
    /// working from a stale list (a bar that asked for workspace 4 as the
    /// third one was being dropped), and silently activating the *last*
    /// workspace instead would be a switch the user never asked for. Doing
    /// nothing is what the one protocol that can produce this already
    /// promises its clients ("there is no guarantee the workspace will be
    /// actually activated").
    pub(super) fn focus_workspace_index(&mut self, index: usize) {
        if index >= self.workspaces.len() {
            return;
        }
        self.active = index;
        // As in `focus_workspace`: leaving an empty workspace behind drops
        // it, which renumbers everything after it. That is why an index only
        // means anything against the list it was read from.
        self.normalize();
    }

    /// Carries the focused window to the workspace at `index`, and follows
    /// it there.
    ///
    /// Ignoring rather than creating or clamping, like
    /// [`focus_workspace_index`](Self::focus_workspace_index): an index past
    /// the end is a caller working from a stale list, and silently carrying
    /// the window to the *last* workspace instead would be a move the user
    /// never asked for. The window stays where it is. Moving to the
    /// already-active workspace, or with no window focused, likewise does
    /// nothing -- the same early returns the relative
    /// [`move_focused_window_to_workspace`](Self::move_focused_window_to_workspace)
    /// makes at the tree's edge and on an empty workspace.
    ///
    /// Returns the window that moved, if one did.
    pub(super) fn move_focused_window_to_workspace_index(
        &mut self,
        index: usize,
    ) -> Option<WindowId> {
        if index >= self.workspaces.len() || index == self.active {
            return None;
        }
        self.carry_focused_window_to(index)
    }

    /// Carries the focused window to the neighbouring workspace, and follows
    /// it. Returns the window that moved, if one did.
    pub(super) fn move_focused_window_to_workspace(&mut self, dir: Vertical) -> Option<WindowId> {
        let target = step(self.active, self.workspaces.len(), dir == Vertical::Down);
        if target == self.active {
            return None;
        }
        self.carry_focused_window_to(target)
    }

    /// The shared half of both workspace moves: takes the active
    /// workspace's focused window to workspace `target` (a valid index other
    /// than the active one), focused there, and follows it. A floating
    /// window lands on top of the target's floating layer, keeping its
    /// centre (the output is the same); a tiled one as a new column right of
    /// the target's focused column, keeping its width preset.
    fn carry_focused_window_to(&mut self, target: usize) -> Option<WindowId> {
        let source = &mut self.workspaces[self.active];
        let id = if source.floating_has_focus() {
            let top = source.floating.len() - 1;
            let id = source.take_floating(top);
            self.workspaces[target].push_floating(id, true);
            id
        } else {
            let column = source.focused_column()?;
            let (column_index, window_index, preset) =
                (source.focused, column.focused, column.preset);
            let id = source.take(column_index, window_index);
            self.workspaces[target].insert_column(id, preset, true);
            id
        };
        self.active = target;
        self.normalize();
        Some(id)
    }

    /// Takes in workspaces from elsewhere, such as an unplugged output, placing
    /// them before the trailing empty workspace without changing focus.
    pub(super) fn adopt(&mut self, workspaces: impl IntoIterator<Item = Workspace>) {
        let adopted: Vec<Workspace> = workspaces.into_iter().filter(|ws| !ws.is_empty()).collect();
        let at = self.workspaces.len() - 1;
        if self.active >= at {
            self.active += adopted.len();
        }
        self.workspaces.splice(at..at, adopted);
        self.normalize();
    }

    /// Drops empty workspaces other than the active one, and keeps exactly one
    /// empty workspace at the end.
    pub(super) fn normalize(&mut self) {
        let mut kept = Vec::with_capacity(self.workspaces.len() + 1);
        let mut active = 0;
        for (index, ws) in std::mem::take(&mut self.workspaces).into_iter().enumerate() {
            if index == self.active {
                active = kept.len();
                kept.push(ws);
            } else if !ws.is_empty() {
                kept.push(ws);
            }
        }
        if kept.last().is_none_or(|ws| !ws.is_empty()) {
            kept.push(Workspace::default());
        }
        self.workspaces = kept;
        self.active = active;
    }
}

/// What an output's usable area becomes given a reported `usable` and the
/// output's own `area`: their overlap, with an empty axis pinned back to the
/// output's own origin.
///
/// That last part is not cosmetic. [`Rect::intersection`] reports a
/// *non*-overlap at the later of the two starting corners, and one of those
/// corners is ultimately a client's number -- a layer surface whose exclusive
/// zone and margins put it at `i32::MAX` leaves an empty usable area *there*,
/// and `Rect::inset`'s `x + by` then overflows on the next `arrange`. Windows
/// cannot be placed in an empty area whatever its coordinates, so pinning it
/// keeps every coordinate the layout ever sees inside the output that
/// produced it. Found by the randomized invariant test, not by inspection.
fn clamp_usable(usable: Rect, area: Rect) -> Rect {
    let clamped = usable.intersection(area);
    Rect::new(
        if clamped.w == 0 { area.x } else { clamped.x },
        if clamped.h == 0 { area.y } else { clamped.y },
        clamped.w,
        clamped.h,
    )
}

/// Moves `current` one step through `0..len`, stopping at either end.
fn step(current: usize, len: usize, forward: bool) -> usize {
    if len == 0 {
        0
    } else if forward {
        (current + 1).min(len - 1)
    } else {
        current.saturating_sub(1)
    }
}
