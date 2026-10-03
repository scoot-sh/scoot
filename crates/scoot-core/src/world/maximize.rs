//! Maximized: entering, leaving, and the one invariant that keeps it simple.
//!
//! The rules themselves are spelled out once, on
//! [`Action::ToggleMaximize`](crate::Action::ToggleMaximize). This file
//! is how they are kept:
//!
//! - The state is a per-window [`Maximized`] in `WindowState`, so it travels
//!   with nothing: a window that closes takes it along, and no column or
//!   workspace has a second copy to keep in step. It sits beside
//!   [`Fullscreen`](super::tree::Fullscreen) rather than inside it because
//!   the two compose: a window can hold both, fullscreen wins while set,
//!   and leaving fullscreen returns to maximized.
//! - **A maximized window in the strip is always its column's focused
//!   window.** (A floating one has no column; it fills the usable area
//!   while it is its workspace's focused window, like fullscreen -- see
//!   `World::maximized_on`.) That is what makes "at most one per column"
//!   hold without a check of its own, and what lets `arrange` find one on
//!   the window lookups it already makes per column
//!   (`World::column_spans`). Entering refuses a window that is not its
//!   column's focused one; [`World::settle_maximize`] ends the maximized of
//!   any window that stops being it.

use super::World;
use super::tree::{Maximized, Slot};
use crate::types::{OutputId, WindowId};

impl World {
    /// Whether the window is maximized. False for an unknown window.
    ///
    /// This answers the state, not the covering: a maximized window whose
    /// column is focused away, or which is fullscreen, still answers true.
    /// [`World::maximized_on`] answers whether it fills the usable area.
    pub fn is_maximized(&self, id: WindowId) -> bool {
        self.windows
            .get(&id)
            .is_some_and(|window| window.maximized.is_some())
    }

    /// The maximized window filling this output's usable area right now, if
    /// one is: the focused window of the output's active workspace, when
    /// that window is maximized and not fullscreen (fullscreen wins while
    /// set -- see [`World::fullscreen_on`], which answers first).
    ///
    /// `None` for an unknown output. Allocates nothing and walks no window
    /// list -- one output lookup and two map lookups -- because a shell asks
    /// it on per-frame and per-pointer-motion paths.
    pub fn maximized_on(&self, output: OutputId) -> Option<WindowId> {
        let id = self
            .outputs
            .iter()
            .find(|o| o.id == output)?
            .active_workspace()
            .focused_window()?;
        (self.is_maximized(id) && !self.is_fullscreen(id)).then_some(id)
    }

    pub(super) fn toggle_maximize(&mut self) {
        if let Some(id) = self.focused_window() {
            let maximized = !self.is_maximized(id);
            self.set_maximize(id, maximized);
        }
    }

    /// Puts a window into maximized or takes it out -- the one path both the
    /// window's own request and every action go through.
    ///
    /// Entering refuses a window that is not its column's focused window (see
    /// the module doc); an unplaced window (there is no output yet) is alone
    /// in the column it will get, so it may. Entering remembers the scroll of
    /// the workspace it is on, and an explicit leave -- this -- restores it.
    /// Leaving fullscreen restores nothing: the strip is still
    /// maximized-wide, so there is no scroll to put back yet.
    pub(super) fn set_maximize(&mut self, id: WindowId, maximized: bool) {
        if !self.windows.contains_key(&id) || self.is_maximized(id) == maximized {
            return;
        }
        let loc = self.locate(id);
        if maximized {
            let view_x = match loc {
                Some(loc) => {
                    let ws = &self.outputs[loc.output].workspaces[loc.workspace];
                    match loc.slot {
                        Slot::Tiled { column, index } => {
                            if ws.columns[column].focused != index {
                                return;
                            }
                            ws.view_x
                        }
                        // A floating window has no column to be the focused
                        // one of, and entering changes no scroll.
                        Slot::Floating { .. } => ws.view_x,
                    }
                }
                None => 0,
            };
            if let Some(window) = self.windows.get_mut(&id) {
                window.maximized = Some(Maximized { view_x });
            }
        } else {
            let entered = self
                .windows
                .get_mut(&id)
                .and_then(|window| window.maximized.take());
            // Only a column's entering scrolled the strip; a floating
            // window's leaving must not move a strip that has scrolled on
            // its own since.
            if let (Some(loc), Some(entered)) = (loc, entered)
                && matches!(loc.slot, Slot::Tiled { .. })
            {
                self.outputs[loc.output].workspaces[loc.workspace].view_x = entered.view_x;
            }
        }
        if let Some(loc) = loc {
            // Clamps the restored scroll back into range too: the strip may
            // have changed shape while the window was maximized.
            self.fix_view(loc.output);
        }
    }

    /// Ends a window's maximized because the layout moved it, without
    /// restoring the scroll it entered with: the layout that scroll described
    /// is gone. The caller re-scrolls.
    pub(super) fn drop_maximize(&mut self, id: WindowId) {
        if let Some(window) = self.windows.get_mut(&id) {
            window.maximized = None;
        }
    }

    /// Ends the maximized of any window in the focused column that is no
    /// longer that column's focused window -- the half of the module doc's
    /// invariant that entering cannot enforce by itself.
    ///
    /// Only the focused column can have broken it: every action that moves
    /// focus *within* a column (a vertical focus step, consume, focusing a
    /// window by id, a platform-observed focus change) leaves that column as
    /// the focused one, and nothing else changes which window a column has
    /// in focus except closing that window, which cannot hand focus to a
    /// maximized sibling because a sibling never is one. So this is a walk
    /// of one column, not of the tree -- it runs after every action.
    ///
    /// Re-scrolls when it ended one, since the column just got narrower.
    pub(super) fn settle_maximize(&mut self) {
        let Some(column) = self
            .outputs
            .get(self.focused_output)
            .and_then(|output| output.active_workspace().focused_column())
        else {
            return;
        };
        let mut ended = false;
        for (index, id) in column.windows.iter().enumerate() {
            if index == column.focused {
                continue;
            }
            if let Some(window) = self.windows.get_mut(id) {
                ended |= window.maximized.take().is_some();
            }
        }
        if ended {
            self.fix_view(self.focused_output);
        }
    }
}
