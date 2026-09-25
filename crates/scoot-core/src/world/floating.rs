//! Floating: taking a window out of the strip and back, and moving focus
//! between the floating layer and the strip.
//!
//! The rules are spelled out once, on
//! [`Action::ToggleFloating`](crate::Action::ToggleFloating). This file is
//! how they are kept:
//!
//! - The state is split the way fullscreen's is not, because floating has a
//!   stacking order: each workspace's `Workspace::floating` holds the order
//!   (bottom first) and whether the workspace's focus is on it, and each
//!   floating window's `WindowState::floating` holds what travels with the
//!   window (the point it is held by, the size the core asks for, the width
//!   it had as a column). A window is in exactly one of the two places a window can be
//!   -- a column or a floating layer -- and has `floating` set exactly when
//!   it is in a floating layer (or waiting for an output while floating).
//! - **Floating windows never change the strip.** Every column, width,
//!   scroll offset and tiled placement is computed as if the floating layer
//!   were not there; the only strip changes are the ones a window leaving or
//!   joining the strip makes, which is what floating and un-floating are.
//! - The centre is decided once, when the window starts floating, from the
//!   arrangement at that moment -- never on the per-frame `arrange` path,
//!   which stays free of parent lookups. Moving and resizing it (by pointer
//!   or by number) replace it with where the user put it; that is
//!   `floating_move.rs`.

use super::tree::{Anchor, Floating, Slot};
use super::{Location, World};
use crate::geometry::{Point, Size};
use crate::types::WindowId;

/// How many parents [`World::descends_from`] follows before giving up. Far
/// past any real dialog-of-a-dialog chain.
pub(super) const MAX_PARENT_DEPTH: usize = 16;

impl World {
    /// Whether the window floats. False for an unknown window.
    pub fn is_floating(&self, id: WindowId) -> bool {
        self.windows
            .get(&id)
            .is_some_and(|window| window.floating.is_some())
    }

    /// For a floating window, what its placement's size is computed from:
    /// the size it last drew ([`Event::FrameObserved`](crate::Event::FrameObserved)'s
    /// `actual`, zero before it has drawn) and the size the core asks it for,
    /// if any, before clamping. `None` for a window that does not float.
    ///
    /// What a shell compares before and after reporting a floating window's
    /// frame, to re-apply the arrangement only when that frame changed where
    /// the window goes -- one map lookup, cheap on the per-commit path that
    /// asks.
    pub fn floating_size(&self, id: WindowId) -> Option<(Size, Option<Size>)> {
        let window = self.windows.get(&id)?;
        let floating = window.floating?;
        Some((window.drawn, floating.request))
    }

    /// Whether `ancestor` is reached by following `id`'s parents
    /// ([`WindowInfo::parent`](crate::WindowInfo::parent)), at most
    /// [`MAX_PARENT_DEPTH`] steps -- a bound, not a real depth: a client can
    /// name any parent, and a chain that loops must still end. Map lookups
    /// only; `arrange` asks it for each floating window while a fullscreen
    /// window covers the output.
    pub(super) fn descends_from(&self, id: WindowId, ancestor: WindowId) -> bool {
        let mut current = id;
        for _ in 0..MAX_PARENT_DEPTH {
            match self.windows.get(&current).and_then(|w| w.info.parent) {
                Some(parent) if parent == ancestor => return true,
                Some(parent) => current = parent,
                None => return false,
            }
        }
        false
    }

    /// Whether the focused output's active workspace has its focus on its
    /// floating layer -- the question every strip-only action asks first.
    pub(super) fn floating_has_focus(&self) -> bool {
        self.outputs
            .get(self.focused_output)
            .is_some_and(|output| output.active_workspace().floating_has_focus())
    }

    pub(super) fn toggle_floating(&mut self) {
        if let Some(id) = self.focused_window() {
            let floating = !self.is_floating(id);
            self.set_floating(id, floating, None);
        }
    }

    /// Floats a window or puts it back in the strip -- the one path the
    /// platform's map-time decision and every action go through. See the
    /// module doc and [`Action::ToggleFloating`](crate::Action::ToggleFloating).
    ///
    /// `size` is an initial size to ask a newly floating window for (a
    /// window rule's); ignored when un-floating. A fullscreen window leaves
    /// fullscreen first, without restoring the scroll it entered with (the
    /// layout that scroll described is about to change).
    pub(super) fn set_floating(&mut self, id: WindowId, floating: bool, size: Option<Size>) {
        if !self.windows.contains_key(&id) || self.is_floating(id) == floating {
            return;
        }
        if self.is_fullscreen(id) {
            self.drop_fullscreen(id);
        }
        let request = size.map(|size| Size::new(size.w.max(0), size.h.max(0)));
        let request = request.filter(|size| size.w > 0 && size.h > 0);
        match (self.locate(id), floating) {
            // Waiting for an output: nothing to take it out of or put it
            // into yet. `place_window` reads the state when one appears.
            (None, true) => {
                if let Some(window) = self.windows.get_mut(&id) {
                    window.floating = Some(Floating {
                        anchor: None,
                        request,
                        preset: None,
                    });
                }
            }
            (None, false) => {
                if let Some(window) = self.windows.get_mut(&id) {
                    window.floating = None;
                }
            }
            (Some(loc), true) => self.float_window(id, loc, request),
            (Some(loc), false) => self.unfloat_window(id, loc),
        }
    }

    /// Takes a tiled window out of its column and puts it on top of its
    /// workspace's floating layer (directly below the top while another
    /// floating window has focus), focused if it was its workspace's focused
    /// window.
    fn float_window(&mut self, id: WindowId, loc: Location, request: Option<Size>) {
        let Slot::Tiled { column, index } = loc.slot else {
            return;
        };
        // Floating before it ever drew: a window that floats as it maps.
        // Its column went in right of the focused one a moment ago, which
        // scrolled the strip to show it; put that scroll back as well as the
        // focus (`take_for_float`), so the strip is exactly as it was.
        let fresh = self
            .windows
            .get(&id)
            .is_some_and(|w| w.drawn == Size::default());
        let restore_view = self
            .last_open
            .filter(|&(opened, _)| fresh && opened == id)
            .map(|(_, view_x)| view_x);
        let ws = &mut self.outputs[loc.output].workspaces[loc.workspace];
        let focused = ws.focused_window() == Some(id);
        let preset = ws.columns[column].preset;
        ws.take_for_float(column, index);
        if let Some(view_x) = restore_view {
            ws.view_x = view_x;
        }
        ws.push_floating(id, focused);
        if let Some(window) = self.windows.get_mut(&id) {
            window.floating = Some(Floating {
                anchor: None,
                request,
                preset: (!fresh).then_some(preset),
            });
        }
        if restore_view.is_some() {
            self.last_open = None;
        }
        self.fix_view(loc.output);
        // After the strip has settled, so a parent in it is measured where it
        // now is, not where the window's own column had pushed it.
        let anchor = self.parent_centre(id, loc).map(Anchor::centred);
        if let Some(floating) = self.windows.get_mut(&id).and_then(|w| w.floating.as_mut()) {
            floating.anchor = anchor;
        }
    }

    /// Takes a floating window out of the floating layer and inserts it as
    /// a column right of the strip's focused column, at the width it had as a
    /// column (the default for a window that was never one), focused if it
    /// was its workspace's focused window.
    fn unfloat_window(&mut self, id: WindowId, loc: Location) {
        let Slot::Floating { index } = loc.slot else {
            return;
        };
        let last = self.config.column_widths.len() - 1;
        let preset = self
            .windows
            .get_mut(&id)
            .and_then(|window| window.floating.take())
            .and_then(|floating| floating.preset)
            .map_or(self.config.default_column_width, |preset| preset.min(last));
        let ws = &mut self.outputs[loc.output].workspaces[loc.workspace];
        let focused = ws.focused_window() == Some(id);
        ws.take_floating(index);
        ws.insert_column(id, preset, focused);
        self.fix_view(loc.output);
    }

    /// Where a window floated at `loc` is centred, relative to its output's
    /// area origin: the middle of the part of its parent inside the usable
    /// area, when the parent is on the same output's same workspace and some
    /// of it is there (on an inactive workspace, where it would be) --
    /// otherwise `None`, which centres it on the output's usable area. A
    /// parent scrolled out of view has no part there.
    ///
    /// Reads a whole arrangement, which is fine for something that happens
    /// once per float (or per output change) and would not be on `arrange`'s
    /// own per-frame path.
    fn parent_centre(&self, id: WindowId, loc: Location) -> Option<Point> {
        let parent = self.windows.get(&id)?.info.parent?;
        if parent == id {
            return None;
        }
        let parent_loc = self.locate(parent)?;
        if parent_loc.output != loc.output || parent_loc.workspace != loc.workspace {
            return None;
        }
        let output = &self.outputs[loc.output];
        let arrangement = self.arrange();
        let placed = arrangement.get(parent)?;
        if placed.output != output.id {
            return None;
        }
        let shown = placed.rect.intersection(output.usable);
        if shown.w <= 0 || shown.h <= 0 {
            return None;
        }
        Some(Point::new(
            shown
                .x
                .saturating_add(shown.w / 2)
                .saturating_sub(output.area.x),
            shown
                .y
                .saturating_add(shown.h / 2)
                .saturating_sub(output.area.y),
        ))
    }

    /// Re-centres floating windows whose output changed under them -- an
    /// output resized or rescaled, or a window arriving on another one when
    /// its own went away -- on their parent, or the output. A centre is
    /// kept relative to its output's origin, so on a different area it
    /// names somewhere else: a dialog centred on a 4K output would be
    /// clamped into a corner of a 1080p one. Ids that are not floating (or
    /// not placed) are skipped.
    pub(super) fn recentre_floating(&mut self, ids: &[WindowId]) {
        for &id in ids {
            let Some(loc) = self.locate(id) else {
                continue;
            };
            if !matches!(loc.slot, Slot::Floating { .. }) {
                continue;
            }
            let anchor = self.parent_centre(id, loc).map(Anchor::centred);
            if let Some(floating) = self.windows.get_mut(&id).and_then(|w| w.floating.as_mut()) {
                floating.anchor = anchor;
            }
        }
    }

    /// After output `o`'s geometry changed or it adopted workspaces:
    /// scrolls every one of its workspaces (not only the active one, so a
    /// parent on an inactive workspace is measured where it will be) and
    /// re-centres `ids` there.
    pub(super) fn recentre_floating_on_output(&mut self, o: usize, ids: &[WindowId]) {
        let count = self
            .outputs
            .get(o)
            .map_or(0, |output| output.workspaces.len());
        for w in 0..count {
            self.fix_workspace_view(o, w);
        }
        self.recentre_floating(ids);
    }

    /// The floating windows on output `o`, every workspace: what an output
    /// change re-centres. Collected, because re-centring reads the whole
    /// world; output changes are rare, not per frame.
    pub(super) fn floating_on_output(&self, o: usize) -> Vec<WindowId> {
        self.outputs.get(o).map_or_else(Vec::new, |output| {
            output
                .workspaces
                .iter()
                .flat_map(|ws| ws.floating.iter().copied())
                .collect()
        })
    }

    /// Switches the focused workspace's focus between its floating layer
    /// and its strip; nothing when the other side is empty.
    pub(super) fn toggle_floating_focus(&mut self) {
        let o = self.focused_output;
        let Some(output) = self.outputs.get_mut(o) else {
            return;
        };
        let ws = output.active_workspace_mut();
        if ws.floating_has_focus() {
            if !ws.columns.is_empty() {
                ws.floating_focused = false;
            }
        } else if !ws.floating.is_empty() {
            ws.floating_focused = true;
        }
        self.fix_view(o);
    }

    /// A floating window's frame: it is placed at the size it drew, and one
    /// larger than its output's usable area is asked to fit it from now on.
    /// Never teaches a minimum (the strip's business, not a floating
    /// window's).
    pub(super) fn floating_frame(&mut self, id: WindowId, actual: Size) {
        let Some(loc) = self.locate(id) else {
            return;
        };
        let usable = self.outputs[loc.output].usable;
        if usable.w <= 0 || usable.h <= 0 {
            return;
        }
        if actual.w <= usable.w && actual.h <= usable.h {
            return;
        }
        let fit = Size::new(actual.w.min(usable.w), actual.h.min(usable.h));
        if let Some(floating) = self.windows.get_mut(&id).and_then(|w| w.floating.as_mut()) {
            floating.request = Some(fit);
        }
    }

    /// Whether closing the window at `loc` should hand focus to `parent`,
    /// read *before* the window leaves the tree: only for its workspace's
    /// focused floating window, only for a parent on that same workspace --
    /// which therefore keeps a window and survives the removal's
    /// `normalize` at the same index (a dialog alone on an inactive
    /// workspace drops it, and the next workspace would slide into the index
    /// if this were decided afterwards) -- and not for a tiled parent
    /// stacked behind a fullscreen sibling in its column: focusing it would
    /// break the rule that a fullscreen window is its column's focused one,
    /// and ending that fullscreen over a dialog closing would be a surprise.
    /// Focus then stays where the workspace's own flag puts it
    /// (`Workspace::take_floating`): the next floating window, or the strip.
    pub(super) fn refocus_target(
        &self,
        loc: Location,
        parent: Option<WindowId>,
    ) -> Option<WindowId> {
        let parent = parent?;
        let ws = &self.outputs[loc.output].workspaces[loc.workspace];
        let closing = match loc.slot {
            Slot::Floating { index } => ws.floating.get(index).copied(),
            Slot::Tiled { .. } => None,
        }?;
        if ws.focused_window() != Some(closing) || parent == closing {
            return None;
        }
        match ws.slot_of(parent)? {
            Slot::Floating { .. } => Some(parent),
            Slot::Tiled { column, index } => {
                let column = &ws.columns[column];
                let behind_fullscreen = column.focused != index
                    && column
                        .windows
                        .get(column.focused)
                        .is_some_and(|&sibling| self.is_fullscreen(sibling));
                (!behind_fullscreen).then_some(parent)
            }
        }
    }

    /// After the focused floating window of a workspace closed, focuses the
    /// parent [`World::refocus_target`] chose on that workspace.
    pub(super) fn refocus_after_floating_close(
        &mut self,
        output: usize,
        workspace: usize,
        parent: WindowId,
    ) {
        let Some(ws) = self
            .outputs
            .get_mut(output)
            .and_then(|o| o.workspaces.get_mut(workspace))
        else {
            return;
        };
        match ws.slot_of(parent) {
            Some(Slot::Tiled { column, index }) => ws.focus(column, index),
            Some(Slot::Floating { index }) => ws.focus_floating(index),
            None => return,
        }
        self.fix_view(output);
    }
}
