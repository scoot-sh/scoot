//! The keyboard-shortcut table: vim motions for direction, Super as scoot's
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

use scoot_core::{Action, Horizontal, OutputId, Vertical};
use smithay::input::keyboard::{Keysym, ModifiersState};

/// The modifiers a keybinding can require. Lock states (`caps_lock`,
/// `num_lock`) aren't tracked -- a binding should still fire with Caps Lock
/// on -- so this is a deliberate projection of `ModifiersState`, not a
/// wrapper around it.
///
/// `Hash` is for `config.rs`'s bind loader, which groups parsed binds by
/// combo to detect two different combo strings (aliases, modifier order,
/// case) resolving to the same binding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
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
/// separately, rather than as another `scoot_core::Action` variant, because
/// switching kernel virtual terminals is a Linux-session concern with no
/// meaning outside `--tty` -- `scoot_core` stays platform-independent on
/// purpose (this project's stated future goal is a macOS Accessibility-API
/// adapter that depends on that). See `tty::init` for where `ChangeVt`
/// bindings get added to the table, and `State::change_vt` for what handling
/// one actually does.
#[derive(Clone, Debug, PartialEq)]
pub enum Bound {
    Action(Action),
    ChangeVt(u32),
}

/// The digit keys, in order: `DIGITS[i]` is the keycap for workspace index
/// `i` (keycap 1-based, core index 0-based, so `Super+3` is index 2). One
/// shared order for the default table below and its tests, so the two
/// cannot disagree about which key means which workspace.
const DIGITS: [Keysym; 9] = [
    Keysym::_1,
    Keysym::_2,
    Keysym::_3,
    Keysym::_4,
    Keysym::_5,
    Keysym::_6,
    Keysym::_7,
    Keysym::_8,
    Keysym::_9,
];

/// The default vim-motions-plus-Super table, `Vec` rather than a `HashMap`:
/// a couple dozen entries at most, checked once per keypress -- a linear
/// scan is simpler and not measurably slower. `config.rs` builds on this
/// directly (via `insert`/`extend`) to layer a config file's `[binds]` on
/// top, using `scoot_ipc::KeyCombo` to parse the string form.
#[derive(Clone, Debug)]
pub struct Keybindings(Vec<(Modifiers, Keysym, Bound)>);

impl Default for Keybindings {
    fn default() -> Self {
        let mut bindings = vec![
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
            // The chord most tiling compositors (i3, sway, Hyprland) put
            // fullscreen on.
            (SUPER, Keysym::f, Bound::Action(Action::ToggleFullscreen)),
            // The chords i3 and sway give floating: Shift+Space floats or
            // tiles the focused window, bare Space moves focus between the
            // floating windows and the tiled ones.
            (
                SUPER_SHIFT,
                Keysym::space,
                Bound::Action(Action::ToggleFloating),
            ),
            (
                SUPER,
                Keysym::space,
                Bound::Action(Action::ToggleFloatingFocus),
            ),
            (SUPER, Keysym::q, Bound::Action(Action::CloseFocused)),
            // The default terminal; rebind this combo in `[binds]` (see
            // config.rs) to launch something else instead.
            (
                SUPER,
                Keysym::Return,
                Bound::Action(Action::Spawn(vec!["foot".into()])),
            ),
            // Deliberately not Super+Shift+q: that's one slipped Shift away
            // from Super+q (close-focused), and a slip shouldn't be able to
            // end the whole session.
            (SUPER_SHIFT, Keysym::e, Bound::Action(Action::Quit)),
            // Across outputs, for the first two (ids 3+ stay manual -- no key
            // family maps onto eight outputs the way digits map onto nine
            // workspaces). Bare Super focuses, Shift carries the focused
            // window and follows it there: the same split the workspace
            // digits keep, and the combos the docs have shown as the manual
            // example since phase F -- promoting them changes no documented
            // spelling, and a user's own identical bind keeps working by
            // overriding the same combo through `insert`.
            (
                SUPER,
                Keysym::comma,
                Bound::Action(Action::FocusOutput(OutputId(1))),
            ),
            (
                SUPER,
                Keysym::period,
                Bound::Action(Action::FocusOutput(OutputId(2))),
            ),
            (
                SUPER_SHIFT,
                Keysym::comma,
                Bound::Action(Action::MoveFocusedWindowToOutput(OutputId(1))),
            ),
            (
                SUPER_SHIFT,
                Keysym::period,
                Bound::Action(Action::MoveFocusedWindowToOutput(OutputId(2))),
            ),
        ];
        // Numbered workspaces, 1-based on the keycap and 0-based in the
        // core: `Super+3` focuses index 2, `Super+Shift+3` carries the
        // focused window to index 2 and follows it there. Bare Super for
        // focus and Shift for move match the relative pair's convention
        // (`Super+Ctrl+j` focuses, `Super+Ctrl+Shift+j` moves). Targeting a
        // workspace that doesn't exist yet does nothing -- it neither
        // creates one nor clamps to the last one, the same rule
        // `focus-workspace-index N` already keeps over IPC. Matching is by
        // unshifted keysym (see the module doc), so `Super+Shift+1` names
        // the `1` key with Shift held, not `!`.
        for (index, keysym) in DIGITS.into_iter().enumerate() {
            bindings.push((
                SUPER,
                keysym,
                Bound::Action(Action::FocusWorkspaceIndex(index)),
            ));
            bindings.push((
                SUPER_SHIFT,
                keysym,
                Bound::Action(Action::MoveWindowToWorkspaceIndex(index)),
            ));
        }
        Self(bindings)
    }
}

impl Keybindings {
    /// Every binding in table order: the modifiers, the unshifted keysym,
    /// and what the combo is bound to.
    ///
    /// What `--print-default-config` iterates to spell the default `[binds]`
    /// table back out (see `config::default_config_toml`): table order is
    /// the hardcoded `Default` order, so the emitted file is byte-stable
    /// from run to run -- unlike the `HashMap` the file loader reads, which
    /// is why the loader refuses to arbitrate collisions rather than pick a
    /// file-order winner it cannot truthfully name.
    pub fn iter(&self) -> impl Iterator<Item = (Modifiers, Keysym, &Bound)> + '_ {
        self.0
            .iter()
            .map(|(mods, keysym, bound)| (*mods, *keysym, bound))
    }

    /// What `keysym` held with exactly `mods` is bound to, if anything.
    pub fn match_key(&self, keysym: Keysym, mods: Modifiers) -> Option<Bound> {
        self.0
            .iter()
            .find(|(m, k, _)| *m == mods && *k == keysym)
            .map(|(_, _, bound)| bound.clone())
    }

    /// Inserts a binding for this exact combo, in place of whatever (if
    /// anything) was already there, and returns that previous binding.
    ///
    /// This one operation is what both the config loader's rules fall out
    /// of: a user bind overriding a default is just `insert` called with
    /// the table already at its defaults; `--tty`'s VT-switch bindings
    /// overriding a colliding user bind (see `tty::init`) is the same
    /// `insert`, called later. A linear scan, like `match_key`: this table
    /// is too small for a `HashMap` index to be worth maintaining alongside
    /// it.
    pub fn insert(&mut self, mods: Modifiers, keysym: Keysym, bound: Bound) -> Option<Bound> {
        if let Some(slot) = self
            .0
            .iter_mut()
            .find(|(m, k, _)| *m == mods && *k == keysym)
        {
            Some(std::mem::replace(&mut slot.2, bound))
        } else {
            self.0.push((mods, keysym, bound));
            None
        }
    }

    /// Adds bindings on top of the table, each via `insert`. Used by
    /// `--tty` to add `Ctrl+Alt+F1`..`Ctrl+Alt+F12` VT-switch bindings
    /// without the headless/nested backends' tables gaining them too.
    ///
    /// Returns every binding that was displaced, as `(mods, keysym,
    /// previous_bound)` -- `--tty`'s caller must know, since a config-file
    /// bind landing on the same combo as a VT switch would otherwise
    /// silently shadow the one recovery path this project has on real
    /// hardware (see `config.rs`'s module doc).
    pub fn extend(
        &mut self,
        more: impl IntoIterator<Item = (Modifiers, Keysym, Bound)>,
    ) -> Vec<(Modifiers, Keysym, Bound)> {
        more.into_iter()
            .filter_map(|(mods, keysym, bound)| {
                self.insert(mods, keysym, bound)
                    .map(|previous| (mods, keysym, previous))
            })
            .collect()
    }

    /// Whether this table binds exactly what `other` binds: same combos,
    /// same targets.
    ///
    /// What a reload compares its rebuilt table with before swapping it in,
    /// so a reload that changed nothing it was asked to can say so instead
    /// of claiming `binds` as applied. Order-independent by construction:
    /// `[binds]` is read out of a `HashMap` (see `config::apply_binds`),
    /// whose iteration order has no relationship to the file's, so two
    /// tables built from the same file can hold the same binds in a
    /// different `Vec` order. A derived `PartialEq` on the `Vec` would call
    /// those different; this does not. Duplicates cannot hide a difference
    /// either way: `insert` replaces, so neither table holds the same combo
    /// twice, and equal length plus every combo of one matching in the other
    /// is the same mapping.
    ///
    /// Cold path (one comparison per reload request), so the per-combo
    /// `match_key` scan is not load-bearing the way it is per keypress.
    pub fn same_bindings_as(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self.0.iter().all(|(mods, keysym, bound)| {
                other.match_key(*keysym, *mods).as_ref() == Some(bound)
            })
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
        let displaced = table.extend(Keybindings::vt_switch_bindings());
        assert_eq!(table.0.len(), default_len + 12);
        assert!(displaced.is_empty(), "disjoint combos displace nothing");
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
    fn insert_replaces_the_binding_for_the_same_combo_and_returns_the_old_one() {
        let mut table = Keybindings::default();
        let previous = table.insert(SUPER, Keysym::h, Bound::Action(Action::CloseFocused));
        assert_eq!(
            previous,
            Some(Bound::Action(Action::FocusColumn(Horizontal::Left)))
        );
        assert_eq!(
            table.match_key(Keysym::h, SUPER),
            Some(Bound::Action(Action::CloseFocused))
        );
        // Nothing was appended -- the table grew by zero entries.
        assert_eq!(table.0.len(), Keybindings::default().0.len());
    }

    #[test]
    fn insert_on_a_fresh_combo_adds_it_and_returns_none() {
        let mut table = Keybindings::default();
        let default_len = table.0.len();
        let previous = table.insert(CTRL_ALT, Keysym::F2, Bound::ChangeVt(2));
        assert_eq!(previous, None);
        assert_eq!(table.0.len(), default_len + 1);
    }

    #[test]
    fn extend_reports_every_binding_it_displaced() {
        let mut table = Keybindings::default();
        let displaced = table.extend([(SUPER, Keysym::h, Bound::ChangeVt(9))]);
        assert_eq!(
            displaced,
            vec![(
                SUPER,
                Keysym::h,
                Bound::Action(Action::FocusColumn(Horizontal::Left))
            )]
        );
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
    fn numbered_workspaces_are_bound_by_default() {
        // Gap-1 pin: the keycap is 1-based, the core index 0-based, so
        // `Super+3` focuses index 2 and `Super+Shift+3` carries the focused
        // window to index 2.
        let table = Keybindings::default();
        for (i, keysym) in DIGITS.into_iter().enumerate() {
            assert_eq!(
                table.match_key(keysym, SUPER),
                Some(Bound::Action(Action::FocusWorkspaceIndex(i))),
                "Super+{}",
                i + 1
            );
            assert_eq!(
                table.match_key(keysym, SUPER_SHIFT),
                Some(Bound::Action(Action::MoveWindowToWorkspaceIndex(i))),
                "Super+Shift+{}",
                i + 1
            );
        }
    }

    #[test]
    fn output_focus_and_move_are_bound_by_default() {
        // The documented manual binds, promoted: `Super+comma`/`Super+period`
        // focus outputs 1/2, `Super+Shift` carries the focused window there --
        // the same bare-focus / Shift-move split the workspace digits keep.
        // Ids 3+ stay manual (no key family maps onto eight outputs the way
        // digits map onto nine workspaces).
        use scoot_core::OutputId;
        let table = Keybindings::default();
        assert_eq!(
            table.match_key(Keysym::comma, SUPER),
            Some(Bound::Action(Action::FocusOutput(OutputId(1)))),
            "Super+comma"
        );
        assert_eq!(
            table.match_key(Keysym::period, SUPER),
            Some(Bound::Action(Action::FocusOutput(OutputId(2)))),
            "Super+period"
        );
        assert_eq!(
            table.match_key(Keysym::comma, SUPER_SHIFT),
            Some(Bound::Action(Action::MoveFocusedWindowToOutput(OutputId(
                1
            )))),
            "Super+Shift+comma"
        );
        assert_eq!(
            table.match_key(Keysym::period, SUPER_SHIFT),
            Some(Bound::Action(Action::MoveFocusedWindowToOutput(OutputId(
                2
            )))),
            "Super+Shift+period"
        );
    }

    #[test]
    fn super_f_toggles_fullscreen_by_default() {
        let table = Keybindings::default();
        assert_eq!(
            table.match_key(Keysym::f, SUPER),
            Some(Bound::Action(Action::ToggleFullscreen))
        );
    }

    #[test]
    fn same_bindings_survives_build_order_and_spots_a_real_difference() {
        // `[binds]` is read out of a `HashMap`, so two tables built from
        // the same file can hold the same binds in a different `Vec` order
        // -- a derived `PartialEq` would call those different, and a reload
        // would claim `binds` as applied for changing nothing.
        let mut first = Keybindings::default();
        first.insert(SUPER, Keysym::n, Bound::Action(Action::CloseFocused));
        first.insert(SUPER_SHIFT, Keysym::m, Bound::Action(Action::CloseFocused));
        let mut second = Keybindings::default();
        second.insert(SUPER_SHIFT, Keysym::m, Bound::Action(Action::CloseFocused));
        second.insert(SUPER, Keysym::n, Bound::Action(Action::CloseFocused));
        assert!(first.same_bindings_as(&second));
        assert!(second.same_bindings_as(&first));

        let mut different = Keybindings::default();
        different.insert(SUPER, Keysym::n, Bound::Action(Action::Quit));
        assert!(!first.same_bindings_as(&different));
        assert!(!different.same_bindings_as(&first));
        assert!(!Keybindings::default().same_bindings_as(&first));
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
