//! The live keymap as an IPC reply: what `scoot msg binds` answers.
//!
//! The snapshot is derived by diffing the running table against the
//! built-in defaults on every request, so it is always the merged result
//! the compositor uses -- never a re-parse of the file, and never a copy
//! that a reload could leave stale. A cold path (one `binds` request at a
//! time): building the default table per request and the reply's own
//! allocations cost nothing here, and keep this free of stored provenance
//! that reload, `--tty`'s VT layering and config unbinds would all have to
//! keep in agreement.
//!
//! Answered whether or not the session is locked (see `handle_request`):
//! listing what keys do is not a window-management operation.

use scoot_ipc::{BindRow, Response, SkippedBind};

use super::config::{bound_string, combo_string};
use super::keybindings::{Bound, Keybindings};

/// Builds the [`Response::Binds`] reply for the live table plus the config
/// entries that never made it in.
///
/// Row order is the live table's own order (built-ins first in their
/// canonical order, then config binds), followed by one row per default a
/// config unbind removed, in default-table order. Config-added row order
/// follows load order, which has no relationship to the file's line order
/// (see `apply_binds`); the skipped list arrives sorted by combo string.
pub fn snapshot(keybindings: &Keybindings, skipped: &[SkippedBind]) -> Response {
    let defaults = Keybindings::default();
    let mut bindings: Vec<BindRow> = Vec::new();
    for (mods, keysym, bound, flags) in keybindings.iter() {
        let (action, source) = match bound {
            Bound::ChangeVt(_) => (bound_string(bound), "session (VT switch)".to_owned()),
            Bound::Action(_) => match defaults.match_key(keysym, mods) {
                None => (bound_string(bound), "config".to_owned()),
                Some((previous, previous_flags)) => {
                    if previous == *bound && previous_flags == flags {
                        (bound_string(bound), "default".to_owned())
                    } else {
                        (
                            bound_string(bound),
                            format!("config (replaces default: {})", bound_string(&previous)),
                        )
                    }
                }
            },
        };
        bindings.push(BindRow {
            combo: combo_string(mods, keysym),
            action,
            source,
            repeat: flags.repeat,
            allow_when_locked: flags.allow_when_locked,
        });
    }
    // Defaults the live table no longer holds: exactly what a config unbind
    // removed (nothing else deletes rows -- `--tty` only replaces). The
    // `action` names what was removed, so the row reads as a removal rather
    // than a binding.
    for (mods, keysym, bound, _) in defaults.iter() {
        if keybindings.match_key(keysym, mods).is_none() {
            let old = bound_string(bound);
            bindings.push(BindRow {
                combo: combo_string(mods, keysym),
                action: old.clone(),
                source: format!("config (unbinds default: {old})"),
                repeat: false,
                allow_when_locked: false,
            });
        }
    }
    Response::Binds {
        bindings,
        skipped: skipped.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use scoot_core::Action;

    use smithay::input::keyboard::Keysym;

    use super::super::keybindings::{BindFlags, Modifiers};
    use super::*;

    const SUPER: Modifiers = Modifiers {
        super_: true,
        shift: false,
        ctrl: false,
        alt: false,
    };

    fn binds_of(response: &Response) -> &[BindRow] {
        match response {
            Response::Binds { bindings, .. } => bindings,
            other => panic!("expected a binds reply, got {other:?}"),
        }
    }

    /// The reply's rows, owned: `snapshot` returns the whole `Response` by
    /// value, so borrowing rows out of a temporary does not live long
    /// enough -- this moves them out instead.
    fn rows(table: &Keybindings) -> Vec<BindRow> {
        match snapshot(table, &[]) {
            Response::Binds { bindings, .. } => bindings,
            other => panic!("expected a binds reply, got {other:?}"),
        }
    }

    fn row_for<'a>(bindings: &'a [BindRow], combo: &str) -> &'a BindRow {
        bindings
            .iter()
            .find(|row| row.combo == combo)
            .unwrap_or_else(|| panic!("no row for `{combo}`"))
    }

    #[test]
    fn defaults_report_as_default_with_no_flags() {
        let response = snapshot(&Keybindings::default(), &[]);
        let bindings = binds_of(&response);
        assert_eq!(bindings.len(), Keybindings::default().iter().count());
        let row = row_for(bindings, "super+h");
        assert_eq!(row.action, "focus-column left");
        assert_eq!(row.source, "default");
        assert!(!row.repeat);
        assert!(!row.allow_when_locked);
        // Every row spells a combo and an action: nothing empty reaches the
        // reply.
        for row in bindings {
            assert!(!row.combo.is_empty(), "a row with no combo");
            assert!(!row.action.is_empty(), "a row with no action");
            assert!(!row.source.is_empty(), "a row with no source");
        }
    }

    #[test]
    fn an_override_names_what_it_replaced() {
        let mut table = Keybindings::default();
        table.insert(
            SUPER,
            Keysym::h,
            Bound::Action(Action::CloseFocused),
            BindFlags::default(),
        );
        let bindings = rows(&table);
        let row = row_for(&bindings, "super+h");
        assert_eq!(row.action, "close");
        assert_eq!(row.source, "config (replaces default: focus-column left)");
    }

    #[test]
    fn a_fresh_combo_reports_as_config() {
        let mut table = Keybindings::default();
        table.insert(
            SUPER,
            Keysym::z,
            Bound::Action(Action::CloseFocused),
            BindFlags::default(),
        );
        let bindings = rows(&table);
        let row = row_for(&bindings, "super+z");
        assert_eq!(row.action, "close");
        assert_eq!(row.source, "config");
    }

    #[test]
    fn an_unbind_reports_as_a_removal_row() {
        let mut table = Keybindings::default();
        table.remove(SUPER, Keysym::q);
        let bindings = rows(&table);
        let row = row_for(&bindings, "super+q");
        assert_eq!(row.action, "close");
        assert_eq!(row.source, "config (unbinds default: close)");
        assert!(!row.repeat);
        assert!(!row.allow_when_locked);
    }

    #[test]
    fn flags_travel_with_the_row() {
        let mut table = Keybindings::default();
        table.insert(
            Modifiers::default(),
            Keysym::F12,
            Bound::Action(Action::Spawn(vec!["true".into()])),
            BindFlags {
                repeat: true,
                allow_when_locked: true,
            },
        );
        let bindings = rows(&table);
        let row = row_for(&bindings, "F12");
        assert!(row.repeat);
        assert!(row.allow_when_locked);
    }

    #[test]
    fn vt_switches_report_as_session() {
        let mut table = Keybindings::default();
        table.extend(Keybindings::vt_switch_bindings());
        let bindings = rows(&table);
        let row = row_for(&bindings, "ctrl+alt+F1");
        assert_eq!(row.action, "change-vt 1");
        assert_eq!(row.source, "session (VT switch)");
    }

    #[test]
    fn skipped_travels_with_the_reply() {
        let skipped = vec![scoot_ipc::SkippedBind {
            bind: "super+notakey".to_owned(),
            value: "\"close\"".to_owned(),
            reason: "unknown key `notakey`".to_owned(),
        }];
        match snapshot(&Keybindings::default(), &skipped) {
            Response::Binds { skipped: back, .. } => assert_eq!(back, skipped),
            other => panic!("expected a binds reply, got {other:?}"),
        }
    }

    #[test]
    fn show_keymap_reports_as_a_default_row() {
        let bindings = rows(&Keybindings::default());
        let row = row_for(&bindings, "super+shift+slash");
        assert_eq!(row.action, "show-keymap");
        assert_eq!(row.source, "default");
    }
}
