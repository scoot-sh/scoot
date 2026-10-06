//! `scoot msg binds` against a known config, through the real dispatch:
//! a default row, an override row naming what it replaced, an unbind row,
//! and skipped entries with their reasons.
//!
//! The pins, on a live headless session whose table was built from a
//! config file's worth of binds the way startup builds it (defaults plus
//! the file, via `keybindings_for`):
//!
//! 1. a default row reports `default` with no flags;
//! 2. an override row reports `config (replaces default: <old action>)`;
//! 3. an unbind row reports `config (unbinds default: <old action>)`;
//! 4. skipped entries (a parse error, a bad action, a colliding group)
//!    each report their reason;
//! 5. the dedicated `Super+Shift+/` row reports `show-keymap` as `default`.
//!
//! Like every other live-`State` test module here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real listening socket.

use std::collections::HashMap;

use scoot_ipc::{BindRow, Request, Response};

use crate::compositor::config::keybindings_for;
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::Harness;

/// The framebuffer the headless backend renders into. Nothing here reads a
/// pixel; the backend exists so the `State` is a whole session.
const CANVAS: i32 = 200;

/// A live compositor whose keymap was built from `binds` (combo/value
/// pairs, the way they read in a config file) layered over the defaults --
/// exactly what startup hands the session, including the skipped list.
fn fixture_with(binds: &[(&str, &str)]) -> Harness<(), ()> {
    let mut map = HashMap::new();
    for (combo, value) in binds {
        map.insert((*combo).to_owned(), toml::Value::from(*value));
    }
    let (keybindings, skipped) = keybindings_for(&map, false);
    let mut fixture: Harness<(), ()> = Harness::headless(Appearance::default(), CANVAS);
    fixture.state.keybindings = keybindings;
    fixture.state.skipped_binds = skipped;
    fixture
}

/// The `binds` reply for `fixture`'s live state: what `scoot msg binds`
/// answers, through the real [`State::handle_request`] dispatch.
fn reply(fixture: &mut Harness<(), ()>) -> (Vec<BindRow>, Vec<scoot_ipc::SkippedBind>) {
    match fixture.state.handle_request(Request::Binds) {
        Response::Binds { bindings, skipped } => (bindings, skipped),
        other => panic!("`binds` answers a binds reply, got {other:?}"),
    }
}

fn row_for<'a>(bindings: &'a [BindRow], combo: &str) -> &'a BindRow {
    bindings
        .iter()
        .find(|row| row.combo == combo)
        .unwrap_or_else(|| panic!("no row for `{combo}`"))
}

/// Pin 1-3 and 5: the default, the override, the unbind and the keymap key.
#[test]
fn binds_reports_the_live_merged_table() {
    let mut fixture = fixture_with(&[
        ("super+m", "close"),
        ("super+q", "none"),
        ("super+z", "toggle-fullscreen"),
    ]);
    let (bindings, skipped) = reply(&mut fixture);
    assert!(skipped.is_empty(), "{skipped:?}");

    let row = row_for(&bindings, "super+h");
    assert_eq!(row.action, "focus-column left");
    assert_eq!(row.source, "default");
    assert!(!row.repeat);
    assert!(!row.allow_when_locked);

    let row = row_for(&bindings, "super+m");
    assert_eq!(row.action, "close");
    assert_eq!(row.source, "config (replaces default: toggle-maximize)");

    let row = row_for(&bindings, "super+q");
    assert_eq!(row.action, "close");
    assert_eq!(row.source, "config (unbinds default: close)");

    let row = row_for(&bindings, "super+z");
    assert_eq!(row.action, "toggle-fullscreen");
    assert_eq!(row.source, "config");

    let row = row_for(&bindings, "super+shift+slash");
    assert_eq!(row.action, "show-keymap");
    assert_eq!(row.source, "default");
}

/// Pin 4: every skipped shape reports its reason.
#[test]
fn binds_reports_skipped_entries_with_reasons() {
    let mut fixture = fixture_with(&[
        ("super+notakey", "close"),
        ("super+F1", "not-an-action"),
        ("super+F2", "close"),
        ("Super+F2", "quit"),
    ]);
    let (bindings, skipped) = reply(&mut fixture);
    // The colliding pair left the combo unbound, and everything else in the
    // file still applied.
    assert!(
        bindings.iter().all(|row| row.combo != "super+F2"),
        "a colliding group binds nothing"
    );
    assert_eq!(skipped.len(), 4, "{skipped:?}");
    let reason_for = |bind: &str| {
        skipped
            .iter()
            .find(|entry| entry.bind == bind)
            .unwrap_or_else(|| panic!("no skipped entry for `{bind}`"))
            .reason
            .clone()
    };
    assert!(
        reason_for("super+notakey").contains("unknown key"),
        "{}",
        reason_for("super+notakey")
    );
    assert!(
        reason_for("super+F1").contains("not-an-action"),
        "{}",
        reason_for("super+F1")
    );
    for bind in ["super+F2", "Super+F2"] {
        assert!(
            reason_for(bind).contains("same key combination"),
            "{}",
            reason_for(bind)
        );
    }
    // Sorted by combo string, so the reply is stable run to run.
    let mut names: Vec<&str> = skipped.iter().map(|entry| entry.bind.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        skipped
            .iter()
            .map(|entry| entry.bind.as_str())
            .collect::<Vec<_>>(),
        names
    );
}
