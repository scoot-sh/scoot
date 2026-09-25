//! Applying what a platform shell observed.

use super::World;
use super::tree::{Output, Slot, WindowState};
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
            Event::OutputRemoved { id } => self.remove_output(id),
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
        let focused_floating = matches!(loc.slot, Slot::Floating { .. })
            && self.outputs[loc.output].workspaces[loc.workspace].focused_window() == Some(id);
        self.remove_window(loc);
        if focused_floating {
            self.refocus_after_floating_close(loc.output, loc.workspace, window.info.parent);
        }
    }

    /// Adds an output, or updates its area if it is already known.
    fn upsert_output(&mut self, id: OutputId, area: Rect) {
        if let Some(o) = self.output_index(id) {
            self.outputs[o].set_area(area);
            self.fix_view(o);
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

    /// The removed output's workspaces join the focused output, after its own,
    /// without moving focus. With no output left to take them, their windows
    /// wait, as single columns, for the next output.
    fn remove_output(&mut self, id: OutputId) {
        let Some(o) = self.output_index(id) else {
            return;
        };
        let removed = self.outputs.remove(o);
        if self.focused_output > o {
            self.focused_output -= 1;
        }
        self.focused_output = self
            .focused_output
            .min(self.outputs.len().saturating_sub(1));
        let target = self.focused_output;
        match self.outputs.get_mut(target) {
            Some(output) => {
                output.adopt(removed.workspaces);
                self.fix_view(target);
            }
            None => self.unplaced.extend(removed.into_windows()),
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
