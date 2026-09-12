//! Conversions into `flexwm_core`, for shells that host the protocol.

use crate::action::{Action, Horizontal, Vertical};

impl From<Horizontal> for flexwm_core::Horizontal {
    fn from(direction: Horizontal) -> Self {
        match direction {
            Horizontal::Left => Self::Left,
            Horizontal::Right => Self::Right,
        }
    }
}

impl From<Vertical> for flexwm_core::Vertical {
    fn from(direction: Vertical) -> Self {
        match direction {
            Vertical::Up => Self::Up,
            Vertical::Down => Self::Down,
        }
    }
}

impl From<Action> for flexwm_core::Action {
    fn from(action: Action) -> Self {
        match action {
            Action::FocusColumn { direction } => Self::FocusColumn(direction.into()),
            Action::FocusWindow { direction } => Self::FocusWindow(direction.into()),
            Action::FocusWindowId { id } => Self::FocusWindowId(flexwm_core::WindowId(id)),
            Action::MoveColumn { direction } => Self::MoveColumn(direction.into()),
            Action::MoveWindow { direction } => Self::MoveWindow(direction.into()),
            Action::ConsumeOrExpel { direction } => Self::ConsumeOrExpel(direction.into()),
            Action::CycleColumnWidth => Self::CycleColumnWidth,
            Action::FocusWorkspace { direction } => Self::FocusWorkspace(direction.into()),
            Action::MoveWindowToWorkspace { direction } => {
                Self::MoveWindowToWorkspace(direction.into())
            }
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
            flexwm_core::Action::from(Action::ConsumeOrExpel {
                direction: Horizontal::Right
            }),
            flexwm_core::Action::ConsumeOrExpel(flexwm_core::Horizontal::Right)
        );
        assert_eq!(
            flexwm_core::Action::from(Action::FocusWindowId { id: 7 }),
            flexwm_core::Action::FocusWindowId(flexwm_core::WindowId(7))
        );
    }
}
