//! Fullscreen: entering, leaving, and the one invariant that keeps it simple.
//!
//! The rules themselves are spelled out once, on
//! [`Action::ToggleFullscreen`](crate::Action::ToggleFullscreen). This file
//! is how they are kept:
//!
//! - The state is a per-window [`Fullscreen`] in `WindowState`, so it travels
//!   with nothing: a window that closes takes it along, and no column or
//!   workspace has a second copy to keep in step.
//! - **A fullscreen window is always its column's focused window.** That is
//!   what makes "at most one per column" hold without a check of its own, and
//!   what lets `arrange` find one on the window lookups it already makes per
//!   column (`World::column_spans`). Entering refuses a window that is not its
//!   column's focused one; [`World::settle_fullscreen`] ends the fullscreen of
//!   any window that stops being it.

use super::World;
use super::tree::{Fullscreen, Slot};
use crate::types::{OutputId, WindowId};

impl World {
    /// Whether the window is fullscreen. False for an unknown window.
    pub fn is_fullscreen(&self, id: WindowId) -> bool {
        self.windows
            .get(&id)
            .is_some_and(|window| window.fullscreen.is_some())
    }

    /// The fullscreen window covering this output right now, if one is: the
    /// focused window of the output's active workspace, when that window is
    /// fullscreen. That is exactly when [`World::arrange`] places it over the
    /// output's whole area and every other window on the output invisible --
    /// what a shell reads to decide what may still be drawn above it, or be
    /// pointed at.
    ///
    /// `None` for an unknown output. Allocates nothing and walks no window
    /// list -- one output lookup and one map lookup -- because a shell asks
    /// it on per-frame and per-pointer-motion paths.
    pub fn fullscreen_on(&self, output: OutputId) -> Option<WindowId> {
        let id = self
            .outputs
            .iter()
            .find(|o| o.id == output)?
            .active_workspace()
            .focused_window()?;
        self.is_fullscreen(id).then_some(id)
    }

    pub(super) fn toggle_fullscreen(&mut self) {
        if let Some(id) = self.focused_window() {
            let fullscreen = !self.is_fullscreen(id);
            self.set_fullscreen(id, fullscreen);
        }
    }

    /// Puts a window into fullscreen or takes it out -- the one path both the
    /// window's own request and every action go through.
    ///
    /// Entering refuses a window that is not its column's focused window (see
    /// the module doc); an unplaced window (there is no output yet) is alone
    /// in the column it will get, so it may. Entering remembers the scroll of
    /// the workspace it is on, and an explicit leave -- this -- restores it.
    pub(super) fn set_fullscreen(&mut self, id: WindowId, fullscreen: bool) {
        if !self.windows.contains_key(&id) || self.is_fullscreen(id) == fullscreen {
            return;
        }
        let loc = self.locate(id);
        if fullscreen {
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
                window.fullscreen = Some(Fullscreen { view_x });
            }
        } else {
            let entered = self
                .windows
                .get_mut(&id)
                .and_then(|window| window.fullscreen.take());
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
            // have changed shape while the window was fullscreen.
            self.fix_view(loc.output);
        }
    }

    /// Ends a window's fullscreen because the layout moved it, without
    /// restoring the scroll it entered with: the layout that scroll described
    /// is gone. The caller re-scrolls.
    pub(super) fn drop_fullscreen(&mut self, id: WindowId) {
        if let Some(window) = self.windows.get_mut(&id) {
            window.fullscreen = None;
        }
    }

    /// Ends the fullscreen of any window in the focused column that is no
    /// longer that column's focused window -- the half of the module doc's
    /// invariant that entering cannot enforce by itself.
    ///
    /// Only the focused column can have broken it: every action that moves
    /// focus *within* a column (a vertical focus step, consume, focusing a
    /// window by id, a platform-observed focus change) leaves that column as
    /// the focused one, and nothing else changes which window a column has
    /// in focus except closing that window, which cannot hand focus to a
    /// fullscreen sibling because a sibling never is one. So this is a walk
    /// of one column, not of the tree -- it runs after every action.
    ///
    /// Re-scrolls when it ended one, since the column just got narrower.
    pub(super) fn settle_fullscreen(&mut self) {
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
                ended |= window.fullscreen.take().is_some();
            }
        }
        if ended {
            self.fix_view(self.focused_output);
        }
    }
}
