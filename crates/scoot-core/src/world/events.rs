//! Applying what a platform shell observed.

use super::World;
use super::reconnect::{AdopterView, Removal};
use super::tree::{Output, WindowState, Workspace};
use crate::geometry::{Rect, Size};
use crate::messages::Event;
use crate::types::{OutputId, WindowId, WindowInfo};

/// Pixels of disagreement ignored when learning minimums, so rounding and
/// fractional scaling don't read as a refusal to shrink.
const FRAME_TOLERANCE: i32 = 2;

impl World {
    /// Apply something a platform shell observed.
    pub fn handle_event(&mut self, event: Event) {
        match event {
            Event::OutputAdded { id, area } | Event::OutputChanged { id, area } => {
                self.upsert_output(id, area);
            }
            Event::OutputUsableAreaChanged { id, area } => self.set_usable_area(id, area),
            Event::OutputRemoved { id } => {
                self.remove_output(id);
            }
            Event::WindowOpened {
                id,
                info,
                output,
                focus,
            } => self.open_window(id, info, output, focus),
            Event::WindowChanged { id, info } => {
                if let Some(window) = self.windows.get_mut(&id) {
                    window.info = info;
                    self.fix_all_views();
                }
            }
            Event::WindowClosed { id } => self.close_window(id),
            Event::FrameObserved {
                id,
                requested,
                actual,
            } => self.learn_from_frame(id, requested, actual),
            Event::FocusObserved { id } => {
                if let Some(loc) = self.locate(id) {
                    self.focus_location(loc);
                    self.settle_fullscreen();
                }
            }
            Event::FullscreenRequested { id, fullscreen } => self.set_fullscreen(id, fullscreen),
            Event::FloatingRequested { id, floating, size } => {
                self.set_floating(id, floating, size);
            }
        }
    }

    /// Forgets a window and takes it out of the tree. A focused floating
    /// window hands focus back to its parent when it can (see
    /// `World::refocus_after_floating_close`).
    fn close_window(&mut self, id: WindowId) {
        let Some(window) = self.windows.remove(&id) else {
            return;
        };
        self.unplaced.retain(|&w| w != id);
        if self.last_open.is_some_and(|(opened, _)| opened == id) {
            self.last_open = None;
        }
        let Some(loc) = self.locate(id) else {
            return;
        };
        // Decided before the removal, which may normalize the workspace list
        // (see `World::refocus_target`).
        let refocus = self.refocus_target(loc, window.info.parent);
        self.remove_window(loc);
        if let Some(parent) = refocus {
            self.refocus_after_floating_close(loc.output, loc.workspace, parent);
        }
    }

    /// Adds an output, or updates its area if it is already known.
    fn upsert_output(&mut self, id: OutputId, area: Rect) {
        if let Some(o) = self.output_index(id) {
            let changed = self.outputs[o].area != area;
            self.outputs[o].set_area(area);
            self.fix_view(o);
            if changed {
                let floating = self.floating_on_output(o);
                self.recentre_floating_on_output(o, &floating);
            }
            return;
        }
        self.outputs.push(Output::new(id, area));
        let o = self.outputs.len() - 1;
        for window in std::mem::take(&mut self.unplaced) {
            self.place_window(window, o, false);
        }
    }

    /// Narrows an output's usable area to what the platform left over.
    ///
    /// Only re-scrolls when the area really changed: a bar that repeats its
    /// exclusive zone on every frame it draws (the common case -- a clock
    /// redraws once a second) must not cost a re-layout each time.
    fn set_usable_area(&mut self, id: OutputId, area: Rect) {
        let Some(o) = self.output_index(id) else {
            return;
        };
        if self.outputs[o].set_usable(area) {
            self.fix_view(o);
        }
    }

    /// The removed output's workspaces join the focused output, after its own.
    /// With no output left to take them, their windows wait, as single
    /// columns, for the next output.
    ///
    /// When the session was looking at the removed output, its view follows:
    /// the adopter activates the adopted workspace that held the focused
    /// window (or the one at the removed output's active position, when
    /// nothing there had focus) and keeps focus on it. The adopter's own
    /// previous workspace keeps its index, one keystroke away. When focus was
    /// elsewhere, nothing on the adopter changes, as before -- a monitor
    /// dropping in standby while the user works on the panel leaves the
    /// panel exactly as it was.
    ///
    /// Answers what it did with them: the adopter, if any, the workspace
    /// index in it the adopted block starts at -- what
    /// [`World::restore_output`](super::World::restore_output) verifies each
    /// window against before moving it back -- the opaque origin the adopted
    /// workspaces were tagged with, and where the adopter was looking before.
    pub(super) fn remove_output(&mut self, id: OutputId) -> Removal {
        let Some(o) = self.output_index(id) else {
            return Removal {
                adopted_by: None,
                adopted_at: 0,
                origin: None,
                adopter_active: None,
            };
        };
        // Decided before the removal: was the session looking at this
        // output, and at which window and workspace.
        let focused_here = self.focused_output == o;
        let focused_wid = focused_here.then(|| self.focused_window()).flatten();
        let removed_active = self.outputs[o].active;
        let removed = self.outputs.remove(o);
        if self.focused_output > o {
            self.focused_output -= 1;
        }
        self.focused_output = self
            .focused_output
            .min(self.outputs.len().saturating_sub(1));
        let target = self.focused_output;
        // Its floating windows, re-centred once they are on the output that
        // takes them (see `World::recentre_floating`); the unplaced ones are
        // when an output appears for them (`World::place_window`).
        let floating: Vec<WindowId> = removed
            .workspaces
            .iter()
            .flat_map(|ws| ws.floating.iter().copied())
            .collect();
        // The origin the adopted workspaces are tagged with, minted only
        // when something actually moves: an empty adoption tags nothing,
        // so it names nothing.
        let origin = removed
            .workspaces
            .iter()
            .any(|ws| !ws.is_empty())
            .then(|| self.mint_origin());
        // Where the removed output was looking, in its own terms: the
        // ordinal of its active workspace among its non-empty ones, for the
        // no-focused-window fallback below.
        let mut nonempty_before = 0;
        let mut nonempty_total = 0;
        for (index, ws) in removed.workspaces.iter().enumerate() {
            if !ws.is_empty() {
                if index < removed_active {
                    nonempty_before += 1;
                }
                nonempty_total += 1;
            }
        }
        match self.outputs.get_mut(target) {
            Some(output) => {
                // Before the adopt: the adopted block starts where the
                // trailing empty workspace is now. Every output ends in
                // exactly one (`Output::normalize`), so this always has one
                // to read.
                let adopted_at = output.workspaces.len() - 1;
                let adopted_by = output.id;
                // The adopter's view before the switch below, read before
                // the adopt moves it.
                let before_switch = output.active;
                output.adopt(removed.workspaces, origin.unwrap_or(0));
                // How many workspaces in front of `end` the switch's
                // normalize will drop: every inactive empty one. The
                // adopted workspaces are never empty, and the switch moves
                // the active workspace into the block, so these are exactly
                // the positions the block and the previous view shift by.
                let dropped_before = |workspaces: &[Workspace], end: usize| {
                    workspaces[..end.min(workspaces.len())]
                        .iter()
                        .filter(|ws| ws.is_empty())
                        .count()
                };
                let mut adopted_here = adopted_at;
                let mut adopter_active = None;
                // The switch itself: the session was looking at the removed
                // output, so the adopter shows the adopted workspace that
                // held the focused window and keeps focus on it. Otherwise
                // nothing on the adopter changes.
                if focused_here && nonempty_total > 0 {
                    // The previous view, for the restore to return to --
                    // recorded against the post-switch list, which is when
                    // the restore reads it. A trailing sitter goes back to
                    // the trailing empty; an emptied workspace the switch
                    // drops has no view to return to; anything else is that
                    // many workspaces before the block.
                    let trailing = output.workspaces.len() - 1;
                    let view = before_switch
                        + if before_switch == adopted_at {
                            nonempty_total
                        } else {
                            0
                        };
                    if view == trailing {
                        adopter_active = Some(AdopterView::Trailing);
                    } else if !output.workspaces[view].is_empty() {
                        let view_here = view - dropped_before(&output.workspaces, view);
                        adopted_here = adopted_at - dropped_before(&output.workspaces, adopted_at);
                        adopter_active = Some(AdopterView::BeforeBlock(adopted_here - view_here));
                    } else {
                        adopted_here = adopted_at - dropped_before(&output.workspaces, adopted_at);
                    }
                    match focused_wid
                        .and_then(|wid| self.locate(wid))
                        .filter(|loc| loc.output == target)
                    {
                        Some(loc) => self.focus_location(loc),
                        // No focused window (the removed output's active
                        // workspace was empty): show the adopted workspace
                        // at the position the session was looking at.
                        None => {
                            let switch_to = adopted_at + nonempty_before.min(nonempty_total - 1);
                            self.outputs[target].focus_workspace_index(switch_to);
                            self.fix_view(target);
                        }
                    }
                }
                self.fix_view(target);
                self.recentre_floating_on_output(target, &floating);
                Removal {
                    adopted_by: Some(adopted_by),
                    adopted_at: adopted_here,
                    origin,
                    adopter_active,
                }
            }
            None => {
                self.unplaced.extend(removed.into_windows());
                Removal {
                    adopted_by: None,
                    adopted_at: 0,
                    origin: None,
                    adopter_active: None,
                }
            }
        }
    }

    fn open_window(
        &mut self,
        id: WindowId,
        info: WindowInfo,
        output: Option<OutputId>,
        focus: bool,
    ) {
        if self.windows.contains_key(&id) {
            return;
        }
        self.windows.insert(id, WindowState::new(info));
        let target = output
            .and_then(|o| self.output_index(o))
            .or_else(|| (!self.outputs.is_empty()).then_some(self.focused_output));
        match target {
            Some(o) => self.place_window(id, o, focus),
            None => self.unplaced.push(id),
        }
    }

    /// Raises a window's learned minimum when it settled larger than it was
    /// asked to be, capped to the usable area of its output.
    ///
    /// Not while the window is fullscreen: a frame sized for the whole output
    /// says nothing about how narrow the window can be in its column, and
    /// learning from one would widen that column for good once it leaves.
    ///
    /// Nor while it floats: a floating window's frame is the size it chose,
    /// not a refusal to take one the strip asked for. What it drew is kept
    /// for every window (`WindowState::drawn`), which is what a floating
    /// window is placed at.
    fn learn_from_frame(&mut self, id: WindowId, requested: Size, actual: Size) {
        let Some(window) = self.windows.get_mut(&id) else {
            return;
        };
        window.drawn = Size::new(actual.w.max(0), actual.h.max(0));
        if window.floating.is_some() {
            // A fullscreen frame is the output's size by design, not a
            // window too large for its usable area.
            if window.fullscreen.is_none() {
                self.floating_frame(id, actual);
            }
            return;
        }
        if self.is_fullscreen(id) {
            return;
        }
        let Some(loc) = self.locate(id) else {
            return;
        };
        // `usable`, not `area`: the cap is "as large as this window could
        // legitimately be", and a window can never legitimately fill the
        // strip a bar reserved.
        let limit = self.outputs[loc.output]
            .usable
            .inset(self.config.gap)
            .size();
        let Some(window) = self.windows.get_mut(&id) else {
            return;
        };
        let learn = |requested: i32, actual: i32, current: i32, limit: i32| {
            if actual > requested + FRAME_TOLERANCE {
                actual.min(limit).max(current)
            } else {
                current
            }
        };
        let learned = Size::new(
            learn(requested.w, actual.w, window.learned_min.w, limit.w),
            learn(requested.h, actual.h, window.learned_min.h, limit.h),
        );
        if learned != window.learned_min {
            window.learned_min = learned;
            self.fix_all_views();
        }
    }
}
