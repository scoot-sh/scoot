//! The layout tree. Outputs hold workspaces, workspaces hold a scrolling strip
//! of columns, and columns hold a stack of windows. Each operation here changes
//! one node and its children; [`World`](super::World) coordinates across nodes.

use crate::geometry::{Rect, Size};
use crate::messages::{Horizontal, Vertical};
use crate::types::{OutputId, WindowId, WindowInfo};

#[derive(Debug)]
pub(super) struct WindowState {
    pub(super) info: WindowInfo,
    /// Minimum size learned from frames the window refused to go below.
    pub(super) learned_min: Size,
}

impl WindowState {
    pub(super) fn new(info: WindowInfo) -> Self {
        Self {
            info,
            learned_min: Size::default(),
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

#[derive(Debug, Default)]
pub(super) struct Workspace {
    pub(super) columns: Vec<Column>,
    pub(super) focused: usize,
    /// Scroll offset into the strip of columns. It lives beside the columns
    /// rather than inside them because layout takes it as an input.
    pub(super) view_x: i32,
}

impl Workspace {
    pub(super) fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    pub(super) fn focused_column(&self) -> Option<&Column> {
        self.columns.get(self.focused)
    }

    pub(super) fn focused_window(&self) -> Option<WindowId> {
        let column = self.focused_column()?;
        column.windows.get(column.focused).copied()
    }

    /// The column and stack index holding `id`.
    pub(super) fn position_of(&self, id: WindowId) -> Option<(usize, usize)> {
        self.columns
            .iter()
            .enumerate()
            .find_map(|(c, column)| column.windows.iter().position(|&w| w == id).map(|i| (c, i)))
    }

    fn into_windows(self) -> impl Iterator<Item = WindowId> {
        self.columns.into_iter().flat_map(|c| c.windows)
    }

    /// Opens `id` as a new column right of focus. It takes focus when asked, or
    /// when it is the only column.
    pub(super) fn insert_column(&mut self, id: WindowId, preset: usize, focus: bool) {
        let at = if self.is_empty() { 0 } else { self.focused + 1 };
        self.columns.insert(at, Column::new(id, preset));
        if focus || self.columns.len() == 1 {
            self.focused = at;
        }
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

    pub(super) fn focus(&mut self, column: usize, index: usize) {
        self.focused = column;
        self.columns[column].focused = index;
    }

    pub(super) fn focus_column(&mut self, dir: Horizontal) {
        self.focused = step(self.focused, self.columns.len(), dir == Horizontal::Right);
    }

    pub(super) fn move_column(&mut self, dir: Horizontal) {
        if self.is_empty() {
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

    /// Joins the neighbouring column, or leaves the current one when it holds
    /// other windows (niri's consume-or-expel).
    pub(super) fn consume_or_expel(&mut self, dir: Horizontal) {
        let c = self.focused;
        let Some(column) = self.columns.get_mut(c) else {
            return;
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
                _ => return,
            };
            let id = self.columns.remove(c).windows[0];
            let target = if target > c { target - 1 } else { target };
            let dest = &mut self.columns[target];
            dest.windows.push(id);
            dest.focused = dest.windows.len() - 1;
            self.focused = target;
        }
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

    /// Carries the focused window to the neighbouring workspace, and follows it.
    pub(super) fn move_focused_window_to_workspace(&mut self, dir: Vertical) {
        let target = step(self.active, self.workspaces.len(), dir == Vertical::Down);
        if target == self.active {
            return;
        }
        let source = &mut self.workspaces[self.active];
        let Some(column) = source.focused_column() else {
            return;
        };
        let (column_index, window_index, preset) = (source.focused, column.focused, column.preset);
        let id = source.take(column_index, window_index);
        self.workspaces[target].insert_column(id, preset, true);
        self.active = target;
        self.normalize();
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
