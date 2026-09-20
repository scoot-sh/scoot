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

/// What one output's workspace list looks like right now, as
/// [`World::workspaces`] reports it.
///
/// Positions, not identities. A workspace has no name and no stable id in
/// this core: it is the `n`-th workspace of an output for exactly as long as
/// the list keeps that shape, and leaving an emptied workspace drops it,
/// which renumbers every one after it. Anything that hands these numbers to a
/// client has to re-read them rather than remember them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Workspaces {
    /// How many workspaces the output has. At least one for any output
    /// [`World::workspaces`] reports, since an output always ends in exactly
    /// one empty workspace -- only [`Workspaces::default`], which stands for
    /// "before there was an output at all", has none.
    pub count: usize,
    /// Which of them is active, always `< count`.
    pub active: usize,
}

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

    /// Replaces the layout tunables on a live world: what a config reload
    /// drives, since nothing else writes `config` after `new`.
    ///
    /// Validated exactly like `new`, so a reloaded gap gets the same clamp
    /// the startup one did. Scroll offsets are then re-normalised, the way
    /// every `reshape` action ends in `fix_view`: a gap change moves usable
    /// edges, and a `view_x` that was correct for the old gap can leave the
    /// focused column off screen under the new one.
    ///
    /// What this deliberately does *not* touch: existing columns keep their
    /// width presets, which is why a reload may only shrink or grow
    /// `column_widths` through the compositor's refusal path -- a preset
    /// past the end of a shorter list would index out of range in `arrange`.
    /// See `reload`'s `layout.column_widths` refusal. The `debug_assert`
    /// below is the backstop: any future caller handing a list shorter than
    /// a live preset fails loudly in test/debug instead of panicking a
    /// release session in `arrange`.
    pub fn set_config(&mut self, config: Config) {
        let config = config.validated();
        debug_assert!(
            self.outputs
                .iter()
                .flat_map(|output| &output.workspaces)
                .flat_map(|workspace| &workspace.columns)
                .all(|column| column.preset < config.column_widths.len()),
            "live column preset past the end of the incoming width list"
        );
        self.config = config;
        self.fix_all_views();
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

    /// One output's usable area, or `None` if it isn't known.
    ///
    /// Beside [`World::usable_areas`] rather than folded into it because the
    /// callers differ in how often they run: this one answers "did the
    /// reserved area actually change?" on every commit a bar makes, which is
    /// once a second for a clock and once a frame for anything animated, and
    /// allocating a `Vec` of every output to answer it would be an
    /// allocation per bar frame.
    pub fn usable_area(&self, id: OutputId) -> Option<Rect> {
        self.outputs.iter().find(|o| o.id == id).map(|o| o.usable)
    }

    /// One output's workspaces: how many it has and which one is active.
    ///
    /// Both numbers are read from the same borrow, so they cannot disagree
    /// with each other the way two separate accessors could -- a caller
    /// publishing them (scoot's `ext-workspace-v1` support does) would
    /// otherwise be able to observe an active index that belongs to a
    /// different count than the one it just read.
    ///
    /// `None` for an output this core doesn't know about.
    pub fn workspaces(&self, id: OutputId) -> Option<Workspaces> {
        let output = self.outputs.iter().find(|o| o.id == id)?;
        Some(Workspaces {
            count: output.workspaces.len(),
            active: output.active,
        })
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
