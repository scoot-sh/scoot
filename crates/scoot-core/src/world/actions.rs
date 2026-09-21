//! Applying what a user or agent asked for.

use super::World;
use super::tree::Output;
use crate::messages::{Action, Effect};

impl World {
    /// Apply something a user or agent asked for. Returns the imperative
    /// leftovers for the shell; placement is read back through
    /// [`World::arrange`].
    pub fn handle_action(&mut self, action: Action) -> Vec<Effect> {
        let presets = self.config.column_widths.len();
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
                self.reshape(|o| o.active_workspace_mut().consume_or_expel(dir));
            }
            Action::CycleColumnWidth => {
                self.forget_learned_widths();
                self.reshape(|o| o.active_workspace_mut().cycle_preset(presets));
            }
            Action::FocusWorkspace(dir) => self.reshape(|o| o.focus_workspace(dir)),
            // The focused output's workspaces, like every other `reshape`
            // action here. With more than one output this needs to say
            // *which* output's workspace list the index counts within -- see
            // `World::workspaces`, which has the same caveat from the reading
            // side.
            Action::FocusWorkspaceIndex(index) => self.reshape(|o| o.focus_workspace_index(index)),
            Action::MoveWindowToWorkspace(dir) => {
                self.reshape(|o| o.move_focused_window_to_workspace(dir));
            }
            Action::MoveWindowToWorkspaceIndex(index) => {
                self.reshape(|o| o.move_focused_window_to_workspace_index(index));
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
        Vec::new()
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
