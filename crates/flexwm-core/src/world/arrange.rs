//! Projecting the tree into frames.

use super::World;
use super::tree::{Column, Output, Workspace};
use crate::geometry::Rect;
use crate::layout;
use crate::types::{OutputId, WindowId};

/// Where one window should be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub id: WindowId,
    pub output: OutputId,
    /// Target frame in global logical coordinates. Windows that aren't visible
    /// still get the frame they *would* have, so a shell can animate from it.
    pub rect: Rect,
    /// False when scrolled out of view or on an inactive workspace.
    pub visible: bool,
}

/// A complete, declarative picture of where everything should be. Shells diff
/// consecutive arrangements and apply the difference.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Arrangement {
    pub placements: Vec<Placement>,
    pub focused: Option<WindowId>,
    pub focused_output: Option<OutputId>,
}

impl Arrangement {
    pub fn get(&self, id: WindowId) -> Option<&Placement> {
        self.placements.iter().find(|p| p.id == id)
    }
}

impl World {
    /// Where every window should be right now. Pure: two calls with nothing
    /// handled in between return the same arrangement.
    pub fn arrange(&self) -> Arrangement {
        let mut placements = Vec::new();
        for output in &self.outputs {
            for (index, ws) in output.workspaces.iter().enumerate() {
                self.place_workspace(output, ws, index == output.active, &mut placements);
            }
        }
        Arrangement {
            placements,
            focused: self.focused_window(),
            focused_output: self.focused_output(),
        }
    }

    fn place_workspace(
        &self,
        output: &Output,
        ws: &Workspace,
        active: bool,
        placements: &mut Vec<Placement>,
    ) {
        let gap = self.config.gap;
        // The output minus whatever the platform reserved (a bar's exclusive
        // zone), then minus the layout gap -- never `output.area`, which is
        // the whole screen and includes the reserved strip. See
        // `tree::Output::usable`.
        let usable = output.usable.inset(gap);
        let widths = self.column_widths(ws, usable.w);
        let (starts, _) = layout::starts(&widths, gap);
        for ((column, start), width) in ws.columns.iter().zip(starts).zip(widths) {
            let x = usable.x + start - ws.view_x;
            let on_screen = x < usable.right() && x + width > usable.x;
            let mut y = usable.y;
            for (&id, height) in column
                .windows
                .iter()
                .zip(self.column_heights(column, usable.h))
            {
                placements.push(Placement {
                    id,
                    output: output.id,
                    rect: Rect::new(x, y, width, height),
                    visible: active && on_screen,
                });
                y += height + gap;
            }
        }
    }

    fn column_widths(&self, ws: &Workspace, available: i32) -> Vec<i32> {
        ws.columns
            .iter()
            .map(|column| {
                let min = column
                    .windows
                    .iter()
                    .filter_map(|id| self.windows.get(id))
                    .map(|w| w.min().w)
                    .max()
                    .unwrap_or(0);
                let proportion = self.config.column_widths[column.preset];
                layout::column_width(available, self.config.gap, proportion, min)
            })
            .collect()
    }

    fn column_heights(&self, column: &Column, available: i32) -> Vec<i32> {
        let mins: Vec<i32> = column
            .windows
            .iter()
            .map(|id| self.windows.get(id).map_or(0, |w| w.min().h))
            .collect();
        let gaps = self.config.gap * (column.windows.len() as i32 - 1);
        layout::distribute(available - gaps, &mins)
    }

    /// Scrolls the active workspace of output `o` so its focused column is in
    /// view.
    pub(super) fn fix_view(&mut self, o: usize) {
        let Some(output) = self.outputs.get(o) else {
            return;
        };
        let available = output.usable.inset(self.config.gap).w;
        let ws = output.active_workspace();
        let view = if ws.is_empty() {
            0
        } else {
            let widths = self.column_widths(ws, available);
            let (starts, strip) = layout::starts(&widths, self.config.gap);
            layout::scroll_into_view(
                ws.view_x,
                available,
                starts[ws.focused],
                widths[ws.focused],
                strip,
            )
        };
        self.outputs[o].active_workspace_mut().view_x = view;
    }

    pub(super) fn fix_all_views(&mut self) {
        for o in 0..self.outputs.len() {
            self.fix_view(o);
        }
    }
}
