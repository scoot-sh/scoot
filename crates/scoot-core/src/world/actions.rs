//! Applying what a user or agent asked for.

use super::World;
use super::tree::Output;
use crate::messages::{Action, Effect};
use crate::types::WindowId;

impl World {
    /// Apply something a user or agent asked for. Returns the imperative
    /// leftovers for the shell; placement is read back through
    /// [`World::arrange`].
    pub fn handle_action(&mut self, action: Action) -> Vec<Effect> {
        let presets = self.config.column_widths.len();
        // With a floating window focused, the strip's own actions have no
        // column to act on (see `Action::ToggleFloating`): the two focus
        // steps act on the floating layer instead, and the rest do nothing.
        if self.floating_has_focus() {
            match action {
                // Leave the floating layer for the strip, without stepping.
                Action::FocusColumn(_) => self.toggle_floating_focus(),
                Action::FocusWindow(dir) => {
                    self.reshape(|o| o.active_workspace_mut().cycle_floating(dir));
                }
                Action::MoveColumn(_)
                | Action::MoveWindow(_)
                | Action::ConsumeOrExpel(_)
                | Action::CycleColumnWidth
                | Action::SetColumnWidth(_) => {}
                other => return self.handle_any_focus(other, presets),
            }
            self.settle_fullscreen();
            return Vec::new();
        }
        self.handle_any_focus(action, presets)
    }

    /// Every action, applied with the strip's focus -- and the ones that do
    /// not care where focus is, applied either way.
    fn handle_any_focus(&mut self, action: Action, presets: usize) -> Vec<Effect> {
        match action {
            Action::FocusColumn(dir) => {
                self.reshape(|o| o.active_workspace_mut().focus_column(dir))
            }
            Action::MoveColumn(dir) => self.reshape(|o| o.active_workspace_mut().move_column(dir)),
            Action::FocusWindow(dir) => {
                self.reshape(|o| o.active_workspace_mut().focus_window(dir))
            }
            Action::MoveWindow(dir) => self.reshape(|o| o.active_workspace_mut().move_window(dir)),
            Action::ConsumeOrExpel(dir) => {
                let focused = self.focused_window();
                let mut moved = false;
                self.reshape(|o| moved = o.active_workspace_mut().consume_or_expel(dir));
                if moved && let Some(id) = focused {
                    self.leave_fullscreen_for_move(id);
                }
            }
            Action::CycleColumnWidth => {
                self.forget_learned_widths();
                self.reshape(|o| o.active_workspace_mut().cycle_preset(presets));
            }
            Action::SetColumnWidth(index) => {
                // The range check lives here as well as in `set_preset` so
                // an ignored index changes nothing at all -- not even learned
                // widths: `forget_learned_widths` below would otherwise narrow
                // a column whose preset never moved, making the "no-op" half
                // observable in the arrangement. `reshape`'s own `fix_view`
                // is idempotent, so it needs no such guard (the ignored
                // workspace-index path already runs it the same way).
                if index < presets {
                    self.forget_learned_widths();
                    self.reshape(|o| o.active_workspace_mut().set_preset(index, presets));
                }
            }
            Action::FocusWorkspace(dir) => self.reshape(|o| o.focus_workspace(dir)),
            // The focused output's workspaces, like every other `reshape`
            // action here. With more than one output this needs to say
            // *which* output's workspace list the index counts within -- see
            // `World::workspaces`, which has the same caveat from the reading
            // side.
            Action::FocusWorkspaceIndex(index) => self.reshape(|o| o.focus_workspace_index(index)),
            Action::MoveWindowToWorkspace(dir) => {
                let mut moved = None;
                self.reshape(|o| moved = o.move_focused_window_to_workspace(dir));
                if let Some(id) = moved {
                    self.leave_fullscreen_for_move(id);
                }
            }
            Action::MoveWindowToWorkspaceIndex(index) => {
                let mut moved = None;
                self.reshape(|o| moved = o.move_focused_window_to_workspace_index(index));
                if let Some(id) = moved {
                    self.leave_fullscreen_for_move(id);
                }
            }
            // Cross-output: these cannot go through `reshape`, which only
            // touches the focused output's tree.
            Action::MoveFocusedWindowToOutput(id) => self.move_focused_window_to_output(id),
            Action::FocusOutput(id) => self.focus_output(id),
            Action::FocusWindowId(id) => {
                if let Some(loc) = self.locate(id) {
                    self.focus_location(loc);
                }
            }
            Action::ToggleFullscreen => self.toggle_fullscreen(),
            Action::SetFullscreen { id, fullscreen } => self.set_fullscreen(id, fullscreen),
            Action::ToggleFloating => self.toggle_floating(),
            Action::SetFloating { id, floating } => self.set_floating(id, floating, None),
            Action::ToggleFloatingFocus => self.toggle_floating_focus(),
            Action::CloseFocused => {
                return self
                    .focused_window()
                    .map(Effect::Close)
                    .into_iter()
                    .collect();
            }
            Action::Spawn(command) => return vec![Effect::Spawn(command)],
            Action::Quit => return vec![Effect::Quit],
        }
        self.settle_fullscreen();
        Vec::new()
    }

    /// A move carried the focused window somewhere new: it leaves fullscreen
    /// (without the scroll restore, which described the layout it left), and
    /// the output it is on now re-scrolls around its narrower column.
    fn leave_fullscreen_for_move(&mut self, id: WindowId) {
        if self.is_fullscreen(id) {
            self.drop_fullscreen(id);
            if let Some(loc) = self.locate(id) {
                self.fix_view(loc.output);
            }
        }
    }

    /// Changes the focused output's tree, then scrolls focus back into view.
    fn reshape(&mut self, change: impl FnOnce(&mut Output)) {
        let o = self.focused_output;
        if let Some(output) = self.outputs.get_mut(o) {
            change(output);
            self.fix_view(o);
        }
    }

    /// Choosing a width on purpose overrides any widths learned from frames.
    fn forget_learned_widths(&mut self) {
        let Some(column) = self
            .outputs
            .get(self.focused_output)
            .and_then(|o| o.active_workspace().focused_column())
        else {
            return;
        };
        for id in &column.windows {
            if let Some(window) = self.windows.get_mut(id) {
                window.learned_min.w = 0;
            }
        }
    }
}
