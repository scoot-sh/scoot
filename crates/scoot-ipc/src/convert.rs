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
            Action::FocusWorkspace { direction } => Self::FocusWorkspace(direction.into()),
            Action::FocusWorkspaceIndex { index } => Self::FocusWorkspaceIndex(index),
            Action::MoveWindowToWorkspace { direction } => {
                Self::MoveWindowToWorkspace(direction.into())
            }
            Action::MoveWindowToWorkspaceIndex { index } => Self::MoveWindowToWorkspaceIndex(index),
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
    }
}
