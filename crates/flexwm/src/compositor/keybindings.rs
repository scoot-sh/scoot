//! The keyboard-shortcut table: vim motions for direction, Super as flexwm's
//! own modifier. Matching is pure and unit-testable -- it takes a keysym and
//! the modifiers currently held, nothing live -- following the same shape as
//! `first_free` in `nested/buffers.rs` and `keysym_for_char` in `input.rs`.
//!
//! Matching uses each key's *unshifted* (level 0) keysym rather than the
//! shifted symbol xkb would otherwise produce (e.g. `Keysym::h`, never
//! `Keysym::H`), with Shift tracked as an ordinary modifier instead. That
//! sidesteps case entirely: a table keyed on shifted symbols would need
//! case-insensitive matching to treat `Super+Shift+h` and `Super+Shift+H` as
//! the same binding, and would still be wrong for keys whose shifted form
//! isn't a simple case change (e.g. `1` vs `!`). Binding by physical key
//! identity plus explicit modifiers is both simpler and how real WM
//! keybindings (e.g. niri, sway) already work.

use flexwm_core::{Action, Horizontal, Vertical};
use smithay::input::keyboard::{Keysym, ModifiersState};

/// The modifiers a keybinding can require. Lock states (`caps_lock`,
/// `num_lock`) aren't tracked -- a binding should still fire with Caps Lock
/// on -- so this is a deliberate projection of `ModifiersState`, not a
/// wrapper around it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub super_: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl From<&ModifiersState> for Modifiers {
    fn from(state: &ModifiersState) -> Self {
        Self {
            super_: state.logo,
            shift: state.shift,
            ctrl: state.ctrl,
            alt: state.alt,
        }
    }
}

const fn mods(super_: bool, shift: bool, ctrl: bool, alt: bool) -> Modifiers {
    Modifiers {
        super_,
        shift,
        ctrl,
        alt,
    }
}

const SUPER: Modifiers = mods(true, false, false, false);
const SUPER_SHIFT: Modifiers = mods(true, true, false, false);
const SUPER_ALT: Modifiers = mods(true, false, false, true);
const SUPER_CTRL: Modifiers = mods(true, false, true, false);
const SUPER_CTRL_SHIFT: Modifiers = mods(true, true, true, false);

/// The default vim-motions-plus-Super table. `Vec` rather than a `HashMap`:
/// a couple dozen entries at most, checked once per keypress -- a linear
/// scan is simpler and not measurably slower. Not wired to any string
/// format (like `flexwm_ipc::KeyCombo`) yet -- that seam is for the future
/// config-file loader, and building it before there's a loader to use it
/// would be speculative.
pub struct Keybindings(Vec<(Modifiers, Keysym, Action)>);

impl Default for Keybindings {
    fn default() -> Self {
        Self(vec![
            (SUPER, Keysym::h, Action::FocusColumn(Horizontal::Left)),
            (SUPER, Keysym::l, Action::FocusColumn(Horizontal::Right)),
            (SUPER, Keysym::j, Action::FocusWindow(Vertical::Down)),
            (SUPER, Keysym::k, Action::FocusWindow(Vertical::Up)),
            (SUPER_SHIFT, Keysym::h, Action::MoveColumn(Horizontal::Left)),
            (
                SUPER_SHIFT,
                Keysym::l,
                Action::MoveColumn(Horizontal::Right),
            ),
            (SUPER_SHIFT, Keysym::j, Action::MoveWindow(Vertical::Down)),
            (SUPER_SHIFT, Keysym::k, Action::MoveWindow(Vertical::Up)),
            (
                SUPER_ALT,
                Keysym::h,
                Action::ConsumeOrExpel(Horizontal::Left),
            ),
            (
                SUPER_ALT,
                Keysym::l,
                Action::ConsumeOrExpel(Horizontal::Right),
            ),
            (
                SUPER_CTRL,
                Keysym::j,
                Action::FocusWorkspace(Vertical::Down),
            ),
            (SUPER_CTRL, Keysym::k, Action::FocusWorkspace(Vertical::Up)),
            (
                SUPER_CTRL_SHIFT,
                Keysym::j,
                Action::MoveWindowToWorkspace(Vertical::Down),
            ),
            (
                SUPER_CTRL_SHIFT,
                Keysym::k,
                Action::MoveWindowToWorkspace(Vertical::Up),
            ),
            (SUPER, Keysym::r, Action::CycleColumnWidth),
            (SUPER, Keysym::q, Action::CloseFocused),
            // A placeholder default terminal; becomes configurable once the
            // config-file roadmap item lands.
            (SUPER, Keysym::Return, Action::Spawn(vec!["foot".into()])),
            // Deliberately not Super+Shift+q: that's one slipped Shift away
            // from Super+q (close-focused), and a slip shouldn't be able to
            // end the whole session.
            (SUPER_SHIFT, Keysym::e, Action::Quit),
        ])
    }
}

impl Keybindings {
    /// The action bound to `keysym` held with exactly `mods`, if any.
    pub fn match_key(&self, keysym: Keysym, mods: Modifiers) -> Option<Action> {
        self.0
            .iter()
            .find(|(m, k, _)| *m == mods && *k == keysym)
            .map(|(_, _, action)| action.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_binding_matches_itself() {
        let table = Keybindings::default();
        for (mods, keysym, action) in &table.0 {
            assert_eq!(table.match_key(*keysym, *mods), Some(action.clone()));
        }
    }

    #[test]
    fn an_extra_held_modifier_misses() {
        let table = Keybindings::default();
        // Super+h is bound; Super+Ctrl+h is not -- a superset of modifiers
        // must not match a binding for a subset.
        assert_eq!(
            table.match_key(Keysym::h, mods(true, false, true, false)),
            None
        );
    }

    #[test]
    fn an_unbound_combo_misses() {
        let table = Keybindings::default();
        assert_eq!(table.match_key(Keysym::x, SUPER), None);
        assert_eq!(table.match_key(Keysym::h, Modifiers::default()), None);
    }

    #[test]
    fn modifiers_state_projects_only_the_four_tracked_fields() {
        let state = ModifiersState {
            logo: true,
            shift: true,
            caps_lock: true,
            num_lock: true,
            ..Default::default()
        };
        assert_eq!(
            Modifiers::from(&state),
            mods(true, true, false, false),
            "lock states must not leak into the tracked modifiers"
        );
    }
}
