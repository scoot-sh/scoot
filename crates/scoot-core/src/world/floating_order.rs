//! The order a workspace's floating windows are drawn in, and which of them
//! show.
//!
//! A workspace's floating layer is a stack (`Workspace::floating`, bottom
//! first; focusing a window raises it to the top). It is drawn in that
//! order with one exception: **a window's own floating dialogs are drawn
//! above it**, whatever the stack says. Clicking an app raises it, and
//! without this its modal dialog would go under it: a dialog hidden under
//! the window it blocks looks like a hang (PR #242's review found the
//! fullscreen case, and it holds for any floating parent). The dialog keeps
//! its place in the stack -- focus is still the stack's business, and
//! clicking the parent still focuses the parent -- it is only drawn above.
//!
//! "Its own dialogs" follows parent links between floating windows of the
//! workspace: a dialog of a dialog of an app is drawn above both, at any
//! depth. A link through a window that is not floating on the workspace (a
//! tiled one) lifts nothing -- that window is below every floating window
//! anyway.
//!
//! **How, at any depth, without walking chains.** Each window gets a level:
//! the higher of its own stack index and its parent's level (when its
//! parent floats on the workspace) -- the stack position of the topmost
//! window it must be drawn above -- and a depth, its parent's plus one.
//! Drawing in (level, depth, stack index) order puts every window after its
//! parent (a child's level is at least its parent's, and at an equal level
//! its depth is greater) and lifts a dialog to the level of the highest
//! window of its chain. Windows keep their stack order except for that
//! lift: a lifted dialog draws above everything stacked between it and the
//! window it was lifted to, siblings included -- so a dialog of a lower
//! sibling can draw above a higher sibling of the same parent. The focused
//! window (the top of the stack) is only ever drawn under its own
//! descendants. Levels and depths are
//! computed once per window from its parent's, iteratively (no recursion,
//! however deep the chain), and a parent link that closes a loop is dropped.
//!
//! Cost, because `arrange` runs on every frame: the common case -- no
//! floating window's parent floats, which includes every dialog of a tiled
//! window -- is one map lookup per floating window, then the stack order.
//! A workspace where some floating window's parent floats computes the
//! order in O(n log n) into buffers kept in the `World` and reused, so
//! steady-state frames allocate nothing either way.

use std::cell::RefCell;

use super::World;
use super::arrange::Placement;
use super::floating_move::floating_rect;
use super::tree::{Output, Workspace};
use crate::geometry::{Rect, Size};
use crate::types::WindowId;

/// The buffers the drawing order is computed in, kept between arrangements
/// (see [`World`]'s `order_scratch`): cleared and refilled, never freed, so
/// a workspace with nested floating windows costs no allocation per frame
/// once they have grown to its size.
#[derive(Debug, Default)]
pub(super) struct OrderScratch {
    /// `(id, stack index)`, sorted by id: the stack's index of a window.
    by_id: Vec<(WindowId, usize)>,
    /// Per stack index: the stack index of its parent, when the parent
    /// floats on this workspace (and the link closes no loop).
    parent: Vec<Option<usize>>,
    /// Per stack index: its level, depth, and whether its parent chain
    /// reaches the covering window (see [`World::place_floating`]).
    level: Vec<usize>,
    depth: Vec<usize>,
    dialog_of_covering: Vec<bool>,
    /// Per stack index: not yet visited, being visited, done.
    visit: Vec<Visit>,
    /// The chain being resolved, innermost last.
    path: Vec<usize>,
    /// The drawing order.
    order: Vec<usize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Visit {
    #[default]
    New,
    Open,
    Done,
}

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
        // Whether the window at stack `index` shows; `dialog` is whether it
        // is the covering window's dialog, `above_shown` whether it is drawn
        // above the shown fullscreen window.
        let shown = |id: WindowId, index: usize, dialog: bool, above_shown: bool| match covering {
            // A fullscreen dialog would cover its parent; it waits for focus
            // like any unfocused fullscreen window.
            Some(covering) => id == covering || (dialog && !self.is_fullscreen(id)),
            None if self.is_fullscreen(id) => shown_fullscreen == Some(index),
            None => shown_fullscreen.is_none() || above_shown,
        };
        if !self.lifts_any(ws) {
            for (index, &id) in ws.floating.iter().enumerate() {
                let dialog = covering.is_some_and(|covering| self.descends_from(id, covering));
                let above = shown_fullscreen.is_some_and(|s| index > s);
                self.place_floating_window(
                    output,
                    id,
                    active && shown(id, index, dialog, above),
                    placements,
                );
            }
            return;
        }
        // Reentrancy cannot happen (nothing here arranges); the fallback is
        // there so a future caller costs an allocation, never a panic.
        let mut fresh = OrderScratch::default();
        let mut held = self.order_scratch.try_borrow_mut();
        let scratch = match held.as_deref_mut() {
            Ok(scratch) => scratch,
            Err(_) => &mut fresh,
        };
        self.drawing_order(ws, covering, scratch);
        let mut above = false;
        for &index in &scratch.order {
            let Some(&id) = ws.floating.get(index) else {
                continue;
            };
            let dialog = scratch.dialog_of_covering[index];
            self.place_floating_window(
                output,
                id,
                active && shown(id, index, dialog, above),
                placements,
            );
            above |= shown_fullscreen == Some(index);
        }
    }

    /// Whether any window of the workspace's floating layer has a floating
    /// parent, which is the only way the drawing order can differ from the
    /// stack. One map lookup per window; nothing allocated.
    fn lifts_any(&self, ws: &Workspace) -> bool {
        ws.floating.len() > 1
            && ws.floating.iter().any(|&id| {
                self.windows
                    .get(&id)
                    .and_then(|window| window.info.parent)
                    .is_some_and(|parent| {
                        parent != id
                            && self
                                .windows
                                .get(&parent)
                                .is_some_and(|window| window.floating.is_some())
                    })
            })
    }

    /// Fills `scratch.order` with the workspace's floating stack indices in
    /// drawing order (see the module doc), and `dialog_of_covering` with
    /// which of them descend from `covering`. Every index appears exactly
    /// once, whatever the parent links say: a link that closes a loop is
    /// dropped, so the links form a forest.
    fn drawing_order(
        &self,
        ws: &Workspace,
        covering: Option<WindowId>,
        scratch: &mut OrderScratch,
    ) {
        let stack = &ws.floating;
        let n = stack.len();
        scratch.by_id.clear();
        scratch.by_id.extend(stack.iter().copied().zip(0..));
        scratch.by_id.sort_unstable();
        scratch.parent.clear();
        for (index, &id) in stack.iter().enumerate() {
            let parent = self
                .windows
                .get(&id)
                .and_then(|window| window.info.parent)
                .and_then(|parent| {
                    scratch
                        .by_id
                        .binary_search_by_key(&parent, |&(window, _)| window)
                        .ok()
                        .map(|found| scratch.by_id[found].1)
                })
                .filter(|&parent| parent != index);
            scratch.parent.push(parent);
        }
        scratch.level.clear();
        scratch.level.resize(n, 0);
        scratch.depth.clear();
        scratch.depth.resize(n, 0);
        scratch.dialog_of_covering.clear();
        scratch.dialog_of_covering.resize(n, false);
        scratch.visit.clear();
        scratch.visit.resize(n, Visit::New);
        for start in 0..n {
            // Walk up to the first window already resolved (or a root),
            // then resolve the path from the top down.
            scratch.path.clear();
            let mut current = start;
            while scratch.visit[current] == Visit::New {
                scratch.visit[current] = Visit::Open;
                scratch.path.push(current);
                match scratch.parent[current] {
                    Some(parent) if scratch.visit[parent] == Visit::Open => {
                        // The link closes a loop: drop it.
                        scratch.parent[current] = None;
                        break;
                    }
                    Some(parent) => current = parent,
                    None => break,
                }
            }
            while let Some(index) = scratch.path.pop() {
                let own = stack[index];
                let (level, depth, dialog) = match scratch.parent[index] {
                    Some(parent) => (
                        index.max(scratch.level[parent]),
                        scratch.depth[parent] + 1,
                        covering.is_some_and(|covering| {
                            stack[parent] == covering || scratch.dialog_of_covering[parent]
                        }),
                    ),
                    // A root: its parent (if any) is not floating here, so
                    // whether it descends from the covering window is the
                    // ordinary walk (through tiled windows).
                    None => (
                        index,
                        0,
                        covering.is_some_and(|covering| self.descends_from(own, covering)),
                    ),
                };
                scratch.level[index] = level;
                scratch.depth[index] = depth;
                scratch.dialog_of_covering[index] = dialog;
                scratch.visit[index] = Visit::Done;
            }
        }
        scratch.order.clear();
        scratch.order.extend(0..n);
        let (level, depth) = (&scratch.level, &scratch.depth);
        scratch
            .order
            .sort_unstable_by_key(|&index| (level[index], depth[index], index));
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

/// [`World`]'s field type for the scratch buffers.
pub(super) type OrderScratchCell = RefCell<OrderScratch>;
