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
const CTRL_ALT: Modifiers = mods(false, false, true, true);

/// What a keybinding can be bound to.
///
/// `Action` is the ordinary case: a window-management action, handled
/// identically by every backend via `State::act`. `ChangeVt` exists
/// separately, rather than as another `flexwm_core::Action` variant, because
/// switching kernel virtual terminals is a Linux-session concern with no
/// meaning outside `--tty` -- `flexwm_core` stays platform-independent on
/// purpose (this project's stated future goal is a macOS Accessibility-API
/// adapter that depends on that). See `tty::init` for where `ChangeVt`
/// bindings get added to the table, and `State::change_vt` for what handling
/// one actually does.
#[derive(Clone, Debug, PartialEq)]
pub enum Bound {
    Action(Action),
    ChangeVt(u32),
}

/// The default vim-motions-plus-Super table. `Vec` rather than a `HashMap`:
/// a couple dozen entries at most, checked once per keypress -- a linear
/// scan is simpler and not measurably slower. Not wired to any string
/// format (like `flexwm_ipc::KeyCombo`) yet -- that seam is for the future
/// config-file loader, and building it before there's a loader to use it
/// would be speculative.
pub struct Keybindings(Vec<(Modifiers, Keysym, Bound)>);

impl Default for Keybindings {
    fn default() -> Self {
        Self(vec![
            (
                SUPER,
                Keysym::h,
                Bound::Action(Action::FocusColumn(Horizontal::Left)),
            ),
            (
                SUPER,
                Keysym::l,
                Bound::Action(Action::FocusColumn(Horizontal::Right)),
            ),
            (
                SUPER,
                Keysym::j,
                Bound::Action(Action::FocusWindow(Vertical::Down)),
            ),
            (
                SUPER,
                Keysym::k,
                Bound::Action(Action::FocusWindow(Vertical::Up)),
            ),
            (
                SUPER_SHIFT,
                Keysym::h,
                Bound::Action(Action::MoveColumn(Horizontal::Left)),
            ),
            (
                SUPER_SHIFT,
                Keysym::l,
                Bound::Action(Action::MoveColumn(Horizontal::Right)),
            ),
            (
                SUPER_SHIFT,
                Keysym::j,
                Bound::Action(Action::MoveWindow(Vertical::Down)),
            ),
            (
                SUPER_SHIFT,
                Keysym::k,
                Bound::Action(Action::MoveWindow(Vertical::Up)),
            ),
            (
                SUPER_ALT,
                Keysym::h,
                Bound::Action(Action::ConsumeOrExpel(Horizontal::Left)),
            ),
            (
                SUPER_ALT,
                Keysym::l,
                Bound::Action(Action::ConsumeOrExpel(Horizontal::Right)),
            ),
            (
                SUPER_CTRL,
                Keysym::j,
                Bound::Action(Action::FocusWorkspace(Vertical::Down)),
            ),
            (
                SUPER_CTRL,
                Keysym::k,
                Bound::Action(Action::FocusWorkspace(Vertical::Up)),
            ),
            (
                SUPER_CTRL_SHIFT,
                Keysym::j,
                Bound::Action(Action::MoveWindowToWorkspace(Vertical::Down)),
            ),
            (
                SUPER_CTRL_SHIFT,
                Keysym::k,
                Bound::Action(Action::MoveWindowToWorkspace(Vertical::Up)),
            ),
            (SUPER, Keysym::r, Bound::Action(Action::CycleColumnWidth)),
            (SUPER, Keysym::q, Bound::Action(Action::CloseFocused)),
            // A placeholder default terminal; becomes configurable once the
            // config-file roadmap item lands.
            (
                SUPER,
                Keysym::Return,
                Bound::Action(Action::Spawn(vec!["foot".into()])),
            ),
            // Deliberately not Super+Shift+q: that's one slipped Shift away
            // from Super+q (close-focused), and a slip shouldn't be able to
            // end the whole session.
            (SUPER_SHIFT, Keysym::e, Bound::Action(Action::Quit)),
        ])
    }
}

impl Keybindings {
    /// What `keysym` held with exactly `mods` is bound to, if anything.
    pub fn match_key(&self, keysym: Keysym, mods: Modifiers) -> Option<Bound> {
        self.0
            .iter()
            .find(|(m, k, _)| *m == mods && *k == keysym)
            .map(|(_, _, bound)| bound.clone())
    }

    /// Adds bindings on top of the default table. Used only by `--tty`, to
    /// add `Ctrl+Alt+F1`..`Ctrl+Alt+F12` VT-switch bindings (see
    /// `tty::init`) without the headless/nested backends' tables gaining
    /// them too -- `Keybindings::default()` alone must stay exactly what it
    /// was before this existed.
    pub fn extend(&mut self, more: impl IntoIterator<Item = (Modifiers, Keysym, Bound)>) {
        self.0.extend(more);
    }

    /// `Ctrl+Alt+F1`..`Ctrl+Alt+F12`, bound to switching to VT 1..12.
    /// `--tty`-only (see `extend`'s doc); a free function rather than a
    /// method so it has no dependency on an existing `Keybindings` value.
    pub fn vt_switch_bindings() -> Vec<(Modifiers, Keysym, Bound)> {
        const FUNCTION_KEYS: [Keysym; 12] = [
            Keysym::F1,
            Keysym::F2,
            Keysym::F3,
            Keysym::F4,
            Keysym::F5,
            Keysym::F6,
            Keysym::F7,
            Keysym::F8,
            Keysym::F9,
            Keysym::F10,
            Keysym::F11,
            Keysym::F12,
        ];
        FUNCTION_KEYS
            .into_iter()
            .enumerate()
            .map(|(i, keysym)| (CTRL_ALT, keysym, Bound::ChangeVt(i as u32 + 1)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_binding_matches_itself() {
        let table = Keybindings::default();
        for (mods, keysym, bound) in &table.0 {
            assert_eq!(table.match_key(*keysym, *mods), Some(bound.clone()));
        }
    }

    #[test]
    fn extend_adds_bindings_without_touching_the_defaults() {
        let mut table = Keybindings::default();
        let default_len = table.0.len();
        table.extend(Keybindings::vt_switch_bindings());
        assert_eq!(table.0.len(), default_len + 12);
        assert_eq!(
            table.match_key(Keysym::F2, CTRL_ALT),
            Some(Bound::ChangeVt(2))
        );
        // Every original binding is still there, unchanged.
        for (mods, keysym, bound) in Keybindings::default().0 {
            assert_eq!(table.match_key(keysym, mods), Some(bound));
        }
    }

    #[test]
    fn vt_switch_bindings_cover_f1_through_f12_in_order() {
        let bindings = Keybindings::vt_switch_bindings();
        assert_eq!(bindings.len(), 12);
        for (i, (mods, _, bound)) in bindings.iter().enumerate() {
            assert_eq!(*mods, CTRL_ALT);
            assert_eq!(*bound, Bound::ChangeVt(i as u32 + 1));
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
