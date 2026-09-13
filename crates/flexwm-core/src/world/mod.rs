//! The single writer of window-management state.
//!
//! The public surface is spread over a few files by concern:
//! [`World::handle_event`] lives in `events.rs`, [`World::handle_action`] in
//! `actions.rs`, and [`World::arrange`] in `arrange.rs`. The tree they all
//! operate on is in `tree.rs`.

mod actions;
mod arrange;
mod events;
mod tree;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

pub use arrange::{Arrangement, Placement};

use crate::config::Config;
use crate::geometry::Rect;
use crate::types::{OutputId, WindowId, WindowInfo};
use tree::{Output, WindowState};

/// Where a window sits in the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Location {
    output: usize,
    workspace: usize,
    column: usize,
    index: usize,
}

/// All window-management state, for every output.
#[derive(Debug, Default)]
pub struct World {
    config: Config,
    windows: HashMap<WindowId, WindowState>,
    outputs: Vec<Output>,
    focused_output: usize,
    /// Windows that opened, or were orphaned, while there was no output.
    unplaced: Vec<WindowId>,
}

impl World {
    pub fn new(config: Config) -> Self {
        Self {
            config: config.validated(),
            ..Self::default()
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn focused_window(&self) -> Option<WindowId> {
        self.outputs
            .get(self.focused_output)?
            .active_workspace()
            .focused_window()
    }

    pub fn focused_output(&self) -> Option<OutputId> {
        self.outputs.get(self.focused_output).map(|o| o.id)
    }

    pub fn window_info(&self, id: WindowId) -> Option<&WindowInfo> {
        self.windows.get(&id).map(|w| &w.info)
    }

    /// Every known window, ordered by id.
    pub fn windows(&self) -> Vec<(WindowId, &WindowInfo)> {
        let mut all: Vec<_> = self.windows.iter().map(|(&id, w)| (id, &w.info)).collect();
        all.sort_by_key(|(id, _)| *id);
        all
    }

    /// Every output and its *whole* area, reserved edges included -- this is
    /// the screen as a platform or an agent describes it. What windows are
    /// actually laid out within is [`World::usable_areas`].
    pub fn outputs(&self) -> Vec<(OutputId, Rect)> {
        self.outputs.iter().map(|o| (o.id, o.area)).collect()
    }

    /// Every output's usable area -- its whole area minus whatever the
    /// platform reserved at the edges (see
    /// [`Event::OutputUsableAreaChanged`](crate::Event::OutputUsableAreaChanged))
    /// -- in the same order as [`World::outputs`].
    ///
    /// This is the bound a shell should measure a window against; identical
    /// to `outputs`'s rectangles until something reserves space.
    pub fn usable_areas(&self) -> Vec<Rect> {
        self.outputs.iter().map(|o| o.usable).collect()
    }

    fn output_index(&self, id: OutputId) -> Option<usize> {
        self.outputs.iter().position(|o| o.id == id)
    }

    fn locate(&self, id: WindowId) -> Option<Location> {
        self.outputs.iter().enumerate().find_map(|(output, o)| {
            o.workspaces.iter().enumerate().find_map(|(workspace, ws)| {
                ws.position_of(id).map(|(column, index)| Location {
                    output,
                    workspace,
                    column,
                    index,
                })
            })
        })
    }

    /// Opens a window as a new column right of focus on output `o`.
    fn place_window(&mut self, id: WindowId, o: usize, focus: bool) {
        let preset = self.config.default_column_width;
        let output = &mut self.outputs[o];
        output
            .active_workspace_mut()
            .insert_column(id, preset, focus);
        output.normalize();
        if focus {
            self.focused_output = o;
        }
        self.fix_view(o);
    }

    fn focus_location(&mut self, loc: Location) {
        let output = &mut self.outputs[loc.output];
        output.active = loc.workspace;
        output.active_workspace_mut().focus(loc.column, loc.index);
        output.normalize();
        self.focused_output = loc.output;
        self.fix_view(loc.output);
    }

    /// Takes a window out of the tree, tidying the workspaces behind it.
    fn remove_window(&mut self, loc: Location) {
        let output = &mut self.outputs[loc.output];
        output.workspaces[loc.workspace].take(loc.column, loc.index);
        output.normalize();
        self.fix_view(loc.output);
    }
}
