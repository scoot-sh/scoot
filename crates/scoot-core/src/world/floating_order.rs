//! The order a workspace's floating windows are drawn in, and which of them
//! show.
//!
//! A workspace's floating layer is a stack (`Workspace::floating`, bottom
//! first; focusing a window raises it to the top). It is drawn in that
//! order with one exception: **a window's own floating dialogs -- the
//! floating windows whose parent chain reaches it -- are drawn above it**,
//! whatever the stack says. Clicking an app raises it, and without this its
//! modal dialog would go under it: a dialog hidden under the window it
//! blocks looks like a hang (PR #242's review found the fullscreen case, and
//! it holds for any floating parent). The dialog keeps its place in the
//! stack -- focus is still the stack's business, and clicking the parent
//! still focuses the parent -- it is only drawn above it.
//!
//! The drawing order is a walk of a forest: a window's parent in it is the
//! nearest window of its parent chain that is *above* it in the stack (a
//! dialog already above its parent needs no lifting), the roots are the
//! windows with none, and each window is drawn followed by its children,
//! both in stack order. So a dialog of a dialog stays above the first
//! dialog, and all of them above the app.
//!
//! Cost, because `arrange` runs on every frame: the common case -- no
//! floating window has a floating ancestor, which includes every dialog of a
//! tiled window -- is a walk of each floating window's parent chain (map
//! lookups, usually one or two), then the stack order, allocation-free. Only
//! a workspace where some floating window has a floating ancestor builds the
//! forest, in a few `Vec`s of the stack's length, in O(n log n).

use super::World;
use super::arrange::Placement;
use super::floating::MAX_PARENT_DEPTH;
use super::floating_move::floating_rect;
use super::tree::{Output, Workspace};
use crate::geometry::{Rect, Size};
use crate::types::WindowId;

impl World {
    /// Places a workspace's floating layer in drawing order (see the module
    /// doc, and [`Action::ToggleFloating`](crate::Action::ToggleFloating)
    /// for the rules).
    ///
    /// `covering` is the fullscreen window covering the output, when the
    /// workspace's focused window is one. Everything floating hides under it
    /// except its own floating dialogs, which are drawn above it: a dialog a
    /// fullscreen app opened stays up when the app itself is clicked.
    /// `shown_fullscreen` is the stack index of a floating fullscreen window
    /// shown uncovering (see `place_workspace`): what is drawn below it
    /// hides -- which no longer includes its own dialogs, since they are
    /// drawn above it (PR #242's re-review: a game clicked above its dialog,
    /// then another window focused, hid the dialog under the game).
    pub(super) fn place_floating(
        &self,
        output: &Output,
        ws: &Workspace,
        active: bool,
        covering: Option<WindowId>,
        shown_fullscreen: Option<usize>,
        placements: &mut Vec<Placement>,
    ) {
        // Whether the window at stack `index` shows; `above_shown` is whether
        // it is drawn above the shown fullscreen window.
        let shown = |id: WindowId, index: usize, above_shown: bool| match covering {
            // A fullscreen dialog would cover its parent; it waits for focus
            // like any unfocused fullscreen window.
            Some(covering) => {
                id == covering || (self.descends_from(id, covering) && !self.is_fullscreen(id))
            }
            None if self.is_fullscreen(id) => shown_fullscreen == Some(index),
            None => shown_fullscreen.is_none() || above_shown,
        };
        if !self.lifts_any(ws) {
            for (index, &id) in ws.floating.iter().enumerate() {
                let above = shown_fullscreen.is_some_and(|s| index > s);
                self.place_floating_window(
                    output,
                    id,
                    active && shown(id, index, above),
                    placements,
                );
            }
            return;
        }
        let mut above = false;
        for index in self.drawing_order(ws) {
            let Some(&id) = ws.floating.get(index) else {
                continue;
            };
            self.place_floating_window(output, id, active && shown(id, index, above), placements);
            above |= shown_fullscreen == Some(index);
        }
    }

    /// Whether any window of the workspace's floating layer has a floating
    /// ancestor, which is the only way the drawing order can differ from the
    /// stack. Map lookups along each parent chain; nothing allocated.
    fn lifts_any(&self, ws: &Workspace) -> bool {
        ws.floating.len() > 1 && ws.floating.iter().any(|&id| self.has_floating_ancestor(id))
    }

    fn has_floating_ancestor(&self, id: WindowId) -> bool {
        let mut current = id;
        for _ in 0..MAX_PARENT_DEPTH {
            let Some(parent) = self.windows.get(&current).and_then(|w| w.info.parent) else {
                return false;
            };
            if parent == id {
                return false;
            }
            if self
                .windows
                .get(&parent)
                .is_some_and(|window| window.floating.is_some())
            {
                return true;
            }
            current = parent;
        }
        false
    }

    /// The workspace's floating stack indices in drawing order: the forest
    /// walk of the module doc. Every index appears exactly once, whatever
    /// the parent chains say -- a chain that loops, or runs past
    /// [`MAX_PARENT_DEPTH`], only shortens what is lifted: a window's parent
    /// in the forest is always *above* it in the stack, so the forest has no
    /// cycle to lose a window in.
    fn drawing_order(&self, ws: &Workspace) -> Vec<usize> {
        let stack = &ws.floating;
        let mut by_id: Vec<(WindowId, usize)> = stack.iter().copied().zip(0..).collect();
        by_id.sort_unstable();
        let index_of = |id: WindowId| {
            by_id
                .binary_search_by_key(&id, |&(window, _)| window)
                .ok()
                .map(|found| by_id[found].1)
        };
        // Each window's parent in the forest: the nearest of its ancestors
        // that is above it in the stack.
        let lifter: Vec<Option<usize>> = stack
            .iter()
            .enumerate()
            .map(|(index, &id)| {
                let mut current = id;
                for _ in 0..MAX_PARENT_DEPTH {
                    let parent = self.windows.get(&current).and_then(|w| w.info.parent)?;
                    if parent == id {
                        return None;
                    }
                    if let Some(above) = index_of(parent).filter(|&at| at > index) {
                        return Some(above);
                    }
                    current = parent;
                }
                None
            })
            .collect();
        // Every (parent, child) pair, sorted: each window's children are one
        // run of it, lowest first -- so the walk is O(n log n) however the
        // client shaped its chains, not a scan of the stack per window.
        let mut children: Vec<(usize, usize)> = lifter
            .iter()
            .enumerate()
            .filter_map(|(child, parent)| parent.map(|parent| (parent, child)))
            .collect();
        children.sort_unstable();
        let mut order = Vec::with_capacity(stack.len());
        let mut pending = Vec::new();
        for root in (0..stack.len()).filter(|&index| lifter[index].is_none()) {
            pending.push(root);
            while let Some(index) = pending.pop() {
                order.push(index);
                let first = children.partition_point(|&(parent, _)| parent < index);
                let last = children.partition_point(|&(parent, _)| parent <= index);
                // Pushed highest first, so the lowest pops (and draws, with
                // its own children) first.
                pending.extend(children[first..last].iter().rev().map(|&(_, child)| child));
            }
        }
        order
    }

    /// One floating window's placement. `shown` is whether the workspace's
    /// state lets it show; a window that has not drawn (and was asked for no
    /// size), or an output with no usable area, keeps it hidden anyway. A
    /// fullscreen one is placed over the output's whole area.
    fn place_floating_window(
        &self,
        output: &Output,
        id: WindowId,
        shown: bool,
        placements: &mut Vec<Placement>,
    ) {
        let Some(window) = self.windows.get(&id) else {
            return;
        };
        if window.fullscreen.is_some() {
            let area = output.area;
            let (w, h) = (area.w.max(1), area.h.max(1));
            placements.push(Placement {
                id,
                output: output.id,
                rect: Rect::new(area.x, area.y, w, h),
                visible: shown,
                fullscreen: true,
                floating: true,
                requested: Some(Size::new(w, h)),
            });
            return;
        }
        let placed = floating_rect(window, output);
        placements.push(Placement {
            id,
            output: output.id,
            rect: placed.rect,
            visible: shown && placed.sized && placed.fits,
            fullscreen: false,
            floating: true,
            requested: placed.requested,
        });
    }
}
