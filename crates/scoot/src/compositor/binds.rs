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
/// Row order is stable run to run: default rows in table order (the
/// hardcoded `Default` order), then config rows -- fresh combos and
/// overrides alike -- sorted by combo, then session rows (today only
/// `--tty`'s VT switches, already in `F1`..`F12` order) in table order,
/// followed by one row per default a config unbind removed, in
/// default-table order. Sorting the config rows (rather than keeping table
/// order) is what makes `--json` stable: config binds are read out of a
/// `HashMap` (see `apply_binds`), whose iteration order has no relationship
/// to the file's, so table order for those rows varies run to run. The
/// skipped list arrives sorted by combo string.
pub fn snapshot(keybindings: &Keybindings, skipped: &[SkippedBind]) -> Response {
    let defaults = Keybindings::default();
    let mut default_rows: Vec<BindRow> = Vec::new();
    let mut config_rows: Vec<BindRow> = Vec::new();
    let mut session_rows: Vec<BindRow> = Vec::new();
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
        let row = BindRow {
            combo: combo_string(mods, keysym),
            action,
            source,
            repeat: flags.repeat,
            allow_when_locked: flags.allow_when_locked,
        };
        if row.source == "default" {
            default_rows.push(row);
        } else if row.source.starts_with("config") {
            config_rows.push(row);
        } else {
            session_rows.push(row);
        }
    }
    // Deterministic by construction (see above): an override keeps its
    // mapping wherever it sorts, so two runs of the same file agree.
    config_rows.sort_by(|a, b| a.combo.cmp(&b.combo));
    let mut bindings = default_rows;
    bindings.extend(config_rows);
    bindings.extend(session_rows);
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

    #[test]
    fn config_rows_sort_by_combo_so_json_is_stable() {
        // Config binds arrive out of a `HashMap`, whose iteration order has
        // no relationship to the file's -- so table order for fresh combos
        // varies run to run. The snapshot must not repeat that instability:
        // config rows (fresh and overrides alike) sort by combo, while
        // default and session rows keep their canonical orders.
        let mut table = Keybindings::default();
        // Inserted last-first on purpose: table order here is reverse-alpha.
        table.insert(
            SUPER,
            Keysym::z,
            Bound::Action(Action::CloseFocused),
            BindFlags::default(),
        );
        table.insert(
            SUPER,
            Keysym::y,
            Bound::Action(Action::CloseFocused),
            BindFlags::default(),
        );
        table.insert(
            SUPER,
            Keysym::m,
            Bound::Action(Action::CloseFocused),
            BindFlags::default(),
        );
        table.extend(Keybindings::vt_switch_bindings());
        let bindings = rows(&table);
        let config: Vec<&str> = bindings
            .iter()
            .filter(|row| row.source.starts_with("config"))
            .map(|row| row.combo.as_str())
            .collect();
        assert_eq!(
            config,
            vec!["super+m", "super+y", "super+z"],
            "config rows must sort by combo, not follow load order: {config:?}"
        );
        // Defaults keep their canonical table order around the sorted block
        // (`super+m` is overridden above, so it is a config row now).
        let defaults: Vec<&str> = bindings
            .iter()
            .filter(|row| row.source == "default")
            .map(|row| row.combo.as_str())
            .collect();
        let canonical: Vec<String> = Keybindings::default()
            .iter()
            .map(|(mods, keysym, _, _)| super::super::config::combo_string(mods, keysym))
            .filter(|combo| combo != "super+m")
            .collect();
        assert_eq!(
            defaults,
            canonical.iter().map(String::as_str).collect::<Vec<_>>(),
            "default rows must keep table order"
        );
        // The session rows trail in their own order.
        let session: Vec<&str> = bindings
            .iter()
            .filter(|row| row.source.starts_with("session"))
            .map(|row| row.combo.as_str())
            .collect();
        assert_eq!(session.len(), 12, "{session:?}");
        assert_eq!(session[0], "ctrl+alt+F1");
        assert_eq!(session[11], "ctrl+alt+F12");
    }
}
