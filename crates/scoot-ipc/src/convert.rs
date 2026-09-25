//! Conversions into `scoot_core`, for shells that host the protocol.

use crate::action::{Action, Horizontal, Vertical};

impl From<Horizontal> for scoot_core::Horizontal {
    fn from(direction: Horizontal) -> Self {
        match direction {
            Horizontal::Left => Self::Left,
            Horizontal::Right => Self::Right,
        }
    }
}

impl From<Vertical> for scoot_core::Vertical {
    fn from(direction: Vertical) -> Self {
        match direction {
            Vertical::Up => Self::Up,
            Vertical::Down => Self::Down,
        }
    }
}

impl From<Action> for scoot_core::Action {
    fn from(action: Action) -> Self {
        match action {
            Action::FocusColumn { direction } => Self::FocusColumn(direction.into()),
            Action::FocusWindow { direction } => Self::FocusWindow(direction.into()),
            Action::FocusWindowId { id } => Self::FocusWindowId(scoot_core::WindowId(id)),
            Action::MoveColumn { direction } => Self::MoveColumn(direction.into()),
            Action::MoveWindow { direction } => Self::MoveWindow(direction.into()),
            Action::ConsumeOrExpel { direction } => Self::ConsumeOrExpel(direction.into()),
            Action::CycleColumnWidth => Self::CycleColumnWidth,
            Action::SetColumnWidth { index } => Self::SetColumnWidth(index),
            Action::FocusWorkspace { direction } => Self::FocusWorkspace(direction.into()),
            Action::FocusWorkspaceIndex { index } => Self::FocusWorkspaceIndex(index),
            Action::MoveWindowToWorkspace { direction } => {
                Self::MoveWindowToWorkspace(direction.into())
            }
            Action::MoveWindowToWorkspaceIndex { index } => Self::MoveWindowToWorkspaceIndex(index),
            Action::MoveFocusedWindowToOutput { output } => {
                Self::MoveFocusedWindowToOutput(scoot_core::OutputId(output))
            }
            Action::FocusOutput { output } => Self::FocusOutput(scoot_core::OutputId(output)),
            Action::FocusOutputIndex { index } => Self::FocusOutputIndex(index),
            Action::MoveFocusedWindowToOutputIndex { index } => {
                Self::MoveFocusedWindowToOutputIndex(index)
            }
            Action::ToggleFullscreen => Self::ToggleFullscreen,
            Action::SetFullscreen { id, fullscreen } => Self::SetFullscreen {
                id: scoot_core::WindowId(id),
                fullscreen,
            },
            Action::ToggleFloating => Self::ToggleFloating,
            Action::SetFloating { id, floating } => Self::SetFloating {
                id: scoot_core::WindowId(id),
                floating,
            },
            Action::ToggleFloatingFocus => Self::ToggleFloatingFocus,
            Action::MoveFloating { id, x, y } => Self::MoveFloating {
                id: scoot_core::WindowId(id),
                x,
                y,
            },
            // A size by number keeps the top-left corner: the bottom-right
            // edges move. Past `i32::MAX` saturates; the core clamps to the
            // output long before that matters.
            Action::ResizeFloating { id, width, height } => Self::ResizeFloating {
                id: scoot_core::WindowId(id),
                size: scoot_core::Size::new(
                    i32::try_from(width).unwrap_or(i32::MAX),
                    i32::try_from(height).unwrap_or(i32::MAX),
                ),
                edges: scoot_core::Edges::BOTTOM_RIGHT,
            },
            Action::CloseFocused => Self::CloseFocused,
            Action::Spawn { command } => Self::Spawn(command),
            Action::Quit => Self::Quit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_map_onto_core_actions() {
        assert_eq!(
            scoot_core::Action::from(Action::ConsumeOrExpel {
                direction: Horizontal::Right
            }),
            scoot_core::Action::ConsumeOrExpel(scoot_core::Horizontal::Right)
        );
        assert_eq!(
            scoot_core::Action::from(Action::FocusWindowId { id: 7 }),
            scoot_core::Action::FocusWindowId(scoot_core::WindowId(7))
        );
        assert_eq!(
            scoot_core::Action::from(Action::FocusWorkspaceIndex { index: 2 }),
            scoot_core::Action::FocusWorkspaceIndex(2)
        );
        assert_eq!(
            scoot_core::Action::from(Action::MoveWindowToWorkspaceIndex { index: 3 }),
            scoot_core::Action::MoveWindowToWorkspaceIndex(3)
        );
        assert_eq!(
            scoot_core::Action::from(Action::SetColumnWidth { index: 2 }),
            scoot_core::Action::SetColumnWidth(2)
        );
        assert_eq!(
            scoot_core::Action::from(Action::MoveFocusedWindowToOutput { output: 2 }),
            scoot_core::Action::MoveFocusedWindowToOutput(scoot_core::OutputId(2))
        );
        assert_eq!(
            scoot_core::Action::from(Action::FocusOutput { output: 2 }),
            scoot_core::Action::FocusOutput(scoot_core::OutputId(2))
        );
        assert_eq!(
            scoot_core::Action::from(Action::FocusOutputIndex { index: 1 }),
            scoot_core::Action::FocusOutputIndex(1)
        );
        assert_eq!(
            scoot_core::Action::from(Action::MoveFocusedWindowToOutputIndex { index: 1 }),
            scoot_core::Action::MoveFocusedWindowToOutputIndex(1)
        );
        assert_eq!(
            scoot_core::Action::from(Action::ToggleFullscreen),
            scoot_core::Action::ToggleFullscreen
        );
        assert_eq!(
            scoot_core::Action::from(Action::SetFullscreen {
                id: 7,
                fullscreen: true
            }),
            scoot_core::Action::SetFullscreen {
                id: scoot_core::WindowId(7),
                fullscreen: true
            }
        );
        assert_eq!(
            scoot_core::Action::from(Action::ToggleFloating),
            scoot_core::Action::ToggleFloating
        );
        assert_eq!(
            scoot_core::Action::from(Action::SetFloating {
                id: 7,
                floating: false
            }),
            scoot_core::Action::SetFloating {
                id: scoot_core::WindowId(7),
                floating: false
            }
        );
        assert_eq!(
            scoot_core::Action::from(Action::ToggleFloatingFocus),
            scoot_core::Action::ToggleFloatingFocus
        );
        assert_eq!(
            scoot_core::Action::from(Action::MoveFloating {
                id: 7,
                x: -20,
                y: 40
            }),
            scoot_core::Action::MoveFloating {
                id: scoot_core::WindowId(7),
                x: -20,
                y: 40
            }
        );
        assert_eq!(
            scoot_core::Action::from(Action::ResizeFloating {
                id: 7,
                width: 640,
                height: u32::MAX
            }),
            scoot_core::Action::ResizeFloating {
                id: scoot_core::WindowId(7),
                size: scoot_core::Size::new(640, i32::MAX),
                edges: scoot_core::Edges::BOTTOM_RIGHT
            }
        );
    }
}
