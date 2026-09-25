//! Which windows float as they map -- the heuristics and the rules -- and
//! what such a window is told.

use scoot_ipc::{Request, Response};

use super::*;

impl Fixture {
    /// Points the session at a config file holding `toml` and reloads it.
    fn reload_with(&mut self, dir: &tempfile::TempDir, toml: &str) -> Response {
        let path = dir.path().join("config.toml");
        std::fs::write(&path, toml).expect("a config file");
        self.state.config_path = Some(path);
        let reply = self.state.handle_request(Request::Reload);
        self.settle();
        reply
    }
}

/// The reload's report, with the `[appearance]` fields left out of
/// `applied`: this harness runs a custom appearance, so the first reload of a
/// file without that table also reports it going back to the defaults.
fn reloaded(reply: Response) -> (Vec<String>, Vec<String>) {
    match reply {
        Response::Reloaded { applied, refused } => (
            applied
                .into_iter()
                .filter(|name| !name.starts_with("appearance."))
                .collect(),
            refused,
        ),
        other => panic!("expected a reload report, got {other:?}"),
    }
}

/// The heart of it: a transient window (a parent set before its first
/// commit) floats at the size it chose, centred on its parent, told it is
/// not tiled -- and the strip it would have joined is exactly as it was.
#[test]
fn a_transient_window_floats_centred_on_its_parent_at_its_own_size() {
    let mut fixture = Fixture::new();
    fixture.map(Spec::tiled());
    let strip = fixture.strip();
    let dialog = fixture.map(Spec::dialog_of(0));

    assert!(fixture.floating(dialog));
    let placed = fixture.placement(dialog);
    assert!(placed.visible && placed.floating, "{placed:?}");
    assert_eq!((placed.rect.w, placed.rect.h), NATURAL, "its own size");
    assert_eq!(centre(placed.rect), centre(fixture.placement(0).rect));
    assert_eq!(fixture.strip(), strip, "the strip moved for a dialog");

    // Told first as the column it briefly was (the configure the toplevel's
    // creation sent), then -- in the same flush as its first commit's answer
    // -- that it chooses its own size and is not tiled. It drew for that one.
    let configures = fixture.configures(dialog);
    let last = *configures.last().expect("a configure");
    assert_eq!((last.width, last.height), (0, 0), "{configures:?}");
    assert!(!last.any_tiled && !last.fullscreen, "{configures:?}");

    let snapshot = fixture.snapshot(dialog);
    assert!(snapshot.floating && snapshot.focused, "{snapshot:?}");
    assert_eq!(
        (snapshot.rect.x, snapshot.rect.y),
        (placed.rect.x, placed.rect.y)
    );
    assert!(!fixture.snapshot(0).floating);
}

#[test]
fn a_fixed_size_window_floats_centred_on_its_output() {
    let mut fixture = Fixture::new();
    fixture.map(Spec::tiled());
    let fixed = fixture.map(Spec {
        color: DIALOG_BGRA,
        min: Some((80, 50)),
        max: Some((80, 50)),
        natural: (80, 50),
        ..Spec::tiled()
    });
    assert!(fixture.floating(fixed));
    let placed = fixture.placement(fixed);
    assert_eq!(placed.rect, Rect::new(80, 95, 80, 50));
    assert!(!fixture.last_configure(fixed).any_tiled);
}

/// Not a fixed size: limits equal on one axis only (a max of 0 is
/// "unbounded"), or a minimum with no maximum.
#[test]
fn a_window_fixed_on_one_axis_only_tiles() {
    let mut fixture = Fixture::new();
    fixture.map(Spec::tiled());
    let one_axis = fixture.map(Spec {
        min: Some((80, 50)),
        max: Some((80, 0)),
        ..Spec::tiled()
    });
    assert!(!fixture.floating(one_axis));
    let min_only = fixture.map(Spec {
        min: Some((80, 50)),
        ..Spec::tiled()
    });
    assert!(!fixture.floating(min_only));
}

#[test]
fn a_dialog_hint_floats_a_window_even_without_a_parent() {
    let mut fixture = Fixture::new();
    fixture.map(Spec::tiled());
    let dialog = fixture.map(Spec {
        color: DIALOG_BGRA,
        dialog: true,
        ..Spec::tiled()
    });
    assert!(fixture.floating(dialog));
    assert!(!fixture.last_configure(dialog).any_tiled);
}

#[test]
fn an_ordinary_window_tiles_and_is_told_so() {
    let mut fixture = Fixture::new();
    let window = fixture.map(Spec::tiled());
    assert!(!fixture.floating(window));
    let last = fixture.last_configure(window);
    assert!(last.tiled && last.width > 0, "{last:?}");
}

#[test]
fn auto_off_tiles_even_a_dialog() {
    let mut fixture = Fixture::new();
    let dir = tempfile::tempdir().expect("a temp dir");
    let (applied, refused) = reloaded(fixture.reload_with(&dir, "[floating]\nauto = false\n"));
    assert_eq!(applied, vec!["floating.auto".to_owned()]);
    assert!(refused.is_empty(), "{refused:?}");
    fixture.map(Spec::tiled());
    let dialog = fixture.map(Spec {
        dialog: true,
        min: Some((80, 50)),
        max: Some((80, 50)),
        ..Spec::dialog_of(0)
    });
    assert!(!fixture.floating(dialog));
    assert!(fixture.last_configure(dialog).tiled);
}

#[test]
fn rules_float_by_app_id_or_title_with_a_size_and_float_false_wins() {
    let mut fixture = Fixture::new();
    let dir = tempfile::tempdir().expect("a temp dir");
    let (applied, _) = reloaded(fixture.reload_with(
        &dir,
        "[[window_rule]]\nmatch_app_id = \"rule-float\"\nfloat = true\n\n\
         [[window_rule]]\nmatch_title = \"*Settings*\"\nfloat = true\nsize = [100, 70]\n\n\
         [[window_rule]]\nmatch_app_id = \"keep-tiled\"\nfloat = false\n",
    ));
    assert_eq!(applied, vec!["window_rule".to_owned()]);
    fixture.map(Spec::tiled());

    let by_app_id = fixture.map(Spec {
        app_id: Some("rule-float"),
        ..Spec::tiled()
    });
    assert!(fixture.floating(by_app_id));
    assert_eq!(
        (
            fixture.placement(by_app_id).rect.w,
            fixture.placement(by_app_id).rect.h
        ),
        NATURAL
    );

    // A rule's size is what the window is asked for, and what it draws.
    let by_title = fixture.map(Spec {
        title: Some("App Settings"),
        ..Spec::tiled()
    });
    assert!(fixture.floating(by_title));
    let last = fixture.last_configure(by_title);
    assert_eq!((last.width, last.height), (100, 70));
    assert!(!last.any_tiled);
    let placed = fixture.placement(by_title);
    assert_eq!((placed.rect.w, placed.rect.h), (100, 70));
    assert_eq!(placed.requested, Some(scoot_core::Size::new(100, 70)));

    // A transient window a rule keeps in the strip.
    let kept = fixture.map(Spec {
        app_id: Some("keep-tiled"),
        ..Spec::dialog_of(0)
    });
    assert!(!fixture.floating(kept));
    assert!(fixture.last_configure(kept).tiled);
}

/// Rules decide at map time: a reload reaches the windows mapped after it,
/// not the ones already there -- and a rule it cannot use is refused by name.
#[test]
fn a_reload_changes_the_rules_for_windows_mapped_after_it() {
    let mut fixture = Fixture::new();
    let dir = tempfile::tempdir().expect("a temp dir");
    let before = fixture.map(Spec {
        app_id: Some("later"),
        ..Spec::tiled()
    });
    assert!(!fixture.floating(before));
    let (applied, refused) = reloaded(fixture.reload_with(
        &dir,
        "[[window_rule]]\nmatch_app_id = \"later\"\nfloat = true\n\n\
         [[window_rule]]\nfloat = true\n",
    ));
    assert_eq!(applied, vec!["window_rule".to_owned()]);
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(refused[0].starts_with("window_rule #2 ("), "{refused:?}");
    assert!(!fixture.floating(before), "a mapped window was re-decided");
    let after = fixture.map(Spec {
        app_id: Some("later"),
        ..Spec::tiled()
    });
    assert!(fixture.floating(after));
    // The same file again changes nothing, but still names the broken rule.
    let (applied, refused) = reloaded(fixture.reload_with(
        &dir,
        "[[window_rule]]\nmatch_app_id = \"later\"\nfloat = true\n\n\
         [[window_rule]]\nfloat = true\n",
    ));
    assert!(applied.is_empty(), "{applied:?}");
    assert_eq!(refused.len(), 1);
}

/// The map-time decision is the window's own, not an action: it applies
/// behind the lock screen, so the session is right when it unlocks.
#[test]
fn a_dialog_mapping_while_locked_still_floats() {
    let mut fixture = Fixture::new();
    fixture.map(Spec::tiled());
    fixture.done(Step::LockSession);
    assert!(fixture.state.session_lock.is_locked());
    let dialog = fixture.map(Spec::dialog_of(0));
    assert!(fixture.floating(dialog));
    assert!(!fixture.last_configure(dialog).any_tiled);
}
