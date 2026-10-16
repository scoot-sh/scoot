//! Toolkit tests: the D11 rules against a scripted settings backend,
//! plus the owned-file rules against scratch config homes.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use scoot_look::state::Record;
use scoot_look::{Origin, ResolvedLook};

use super::{
    COLOR_SCHEME_KEY, Dconf, MANAGED_MARK, PREFER_DARK, PREFER_LIGHT, TOOL_KEYS, apply_toolkit,
    apply_toolkit_files, toolkit_off, toolkit_on, wanted_dconf, wanted_files,
};

fn look() -> ResolvedLook {
    let text = super::super::catalog::tests::look_toml("moonrise");
    let raw = scoot_look::parse_look_toml(text.as_bytes()).expect("test look parses");
    let map = BTreeMap::from([("moonrise".to_string(), (raw, Origin::Shipped))]);
    scoot_look::resolve(&map, "moonrise").expect("test look resolves")
}

fn record(toolkit: Option<bool>, wrote: &[(&str, &str)], orig: &[(&str, &str)]) -> Record {
    Record {
        name: "moonrise".to_owned(),
        rendered: "/r/moonrise".to_owned(),
        generation: 1,
        chosen_by: Some("apply".to_owned()),
        declared_default: None,
        toolkit,
        dconf_wrote: wrote
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        dconf_orig: orig
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
    }
}

fn state_dir(tag: &str) -> std::path::PathBuf {
    // A non-private scratch dir: the leaf must not start with the private
    // prefix (`scoot-look-`), and SCOOT_LOOK_STATE is not pointed at it by
    // the tests.
    std::env::temp_dir().join(format!("scoot-toolkit-state-{tag}-{}", std::process::id()))
}

fn link(state: &std::path::Path) -> std::path::PathBuf {
    state.join(scoot_look::state::LINK_NAME)
}

#[derive(Default)]
struct FakeDconf {
    live: HashMap<String, String>,
    writes: RefCell<Vec<(String, String)>>,
    fail_write: bool,
    missing: bool,
    unreadable: bool,
}

impl FakeDconf {
    fn wrote(&self) -> Vec<(String, String)> {
        self.writes.borrow().clone()
    }

    fn wrote_to(&self, key: &str) -> Vec<String> {
        self.writes
            .borrow()
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .collect()
    }
}

impl Dconf for FakeDconf {
    fn available(&self) -> bool {
        !self.missing
    }

    fn read(&self, key: &str) -> Result<Option<String>, String> {
        if self.unreadable {
            return Err("dconf is broken".to_owned());
        }
        Ok(self.live.get(key).cloned())
    }

    fn write(&self, key: &str, value: &str) -> Result<(), String> {
        if self.fail_write {
            return Err("dconf is broken".to_owned());
        }
        self.writes
            .borrow_mut()
            .push((key.to_owned(), value.to_owned()));
        Ok(())
    }

    fn reset(&self, key: &str) -> Result<(), String> {
        self.writes
            .borrow_mut()
            .push((format!("reset {key}"), String::new()));
        Ok(())
    }
}

#[test]
fn toolkit_defaults_on_and_off_is_explicit() {
    assert!(toolkit_on(&record(None, &[], &[])));
    assert!(toolkit_on(&record(Some(true), &[], &[])));
    assert!(!toolkit_on(&record(Some(false), &[], &[])));
}

#[test]
fn wanted_keys_come_from_the_look() {
    // A dark look reads dark; the theme, icons, cursor and font come from
    // their tokens.
    let wanted: HashMap<String, String> =
        wanted_dconf(&look()).expect("wanted").into_iter().collect();
    assert_eq!(wanted.len(), TOOL_KEYS.len());
    assert_eq!(wanted["color-scheme"], PREFER_DARK);
    assert_eq!(wanted["gtk-theme"], "'Adwaita-dark'");
    assert_eq!(wanted["icon-theme"], "'Adwaita'");
    assert_eq!(wanted["cursor-theme"], "'Mocu'");
    assert_eq!(wanted["cursor-size"], "24");
    assert_eq!(wanted["font-name"], "'DejaVu Sans 12'");
}

#[test]
fn unset_keys_are_written_and_recorded() {
    let dconf = FakeDconf::default();
    let mut state = record(None, &[], &[]);
    let notes = apply_toolkit(&state_dir("unset"), &mut state, &look(), &dconf);
    assert_eq!(dconf.wrote().len(), TOOL_KEYS.len());
    assert_eq!(
        dconf.wrote_to(COLOR_SCHEME_KEY),
        vec![PREFER_DARK.to_owned()]
    );
    assert_eq!(
        state.dconf_wrote.get("color-scheme").map(String::as_str),
        Some(PREFER_DARK)
    );
    assert_eq!(
        state.dconf_orig.get("color-scheme").map(String::as_str),
        Some("")
    );
    assert!(
        notes.iter().any(|note| note.contains(PREFER_DARK)),
        "{notes:?}"
    );
}

#[test]
fn a_light_look_writes_prefer_light() {
    let dconf = FakeDconf::default();
    let mut state = record(None, &[], &[]);
    let mut light = look();
    light.polarity = scoot_look::schema::Polarity::Light;
    apply_toolkit(&state_dir("light"), &mut state, &light, &dconf);
    assert_eq!(
        dconf.wrote_to(COLOR_SCHEME_KEY),
        vec![PREFER_LIGHT.to_owned()]
    );
    assert!(
        dconf
            .wrote_to("/org/gnome/desktop/interface/gtk-theme")
            .iter()
            .any(|v| v.contains("Adwaita'") && !v.contains("dark"))
    );
}

#[test]
fn what_the_tool_wrote_is_owned_and_rewritten() {
    let mut live = HashMap::new();
    live.insert(COLOR_SCHEME_KEY.to_owned(), PREFER_DARK.to_owned());
    let dconf = FakeDconf {
        live,
        ..FakeDconf::default()
    };
    let mut state = record(None, &[("color-scheme", PREFER_DARK)], &[]);
    let notes = apply_toolkit(&state_dir("owned"), &mut state, &look(), &dconf);
    // The owned key rewrites; the other five adopt (no record yet).
    assert!(!dconf.wrote_to(COLOR_SCHEME_KEY).is_empty());
    assert!(
        notes.iter().any(|note| note.contains("set to")),
        "{notes:?}"
    );
}

#[test]
fn a_key_someone_else_changed_is_left_alone_and_named() {
    let mut live = HashMap::new();
    live.insert(COLOR_SCHEME_KEY.to_owned(), PREFER_LIGHT.to_owned());
    let dconf = FakeDconf {
        live,
        ..FakeDconf::default()
    };
    let mut state = record(None, &[("color-scheme", PREFER_DARK)], &[]);
    let notes = apply_toolkit(&state_dir("foreign"), &mut state, &look(), &dconf);
    assert!(dconf.wrote_to(COLOR_SCHEME_KEY).is_empty());
    // The foreign key keeps its record (a later `off` still will not
    // touch what the tool did not write last).
    assert_eq!(
        state.dconf_wrote.get("color-scheme").map(String::as_str),
        Some(PREFER_DARK)
    );
    assert!(
        notes
            .iter()
            .any(|note| note.contains("outside `scoot look`")),
        "{notes:?}"
    );
}

#[test]
fn one_foreign_key_leaves_the_other_five_alone() {
    // Ownership is per key: a settings app that owns `gtk-theme` stops
    // only that write, and the other five still land.
    let mut live = HashMap::new();
    live.insert(
        "/org/gnome/desktop/interface/gtk-theme".to_owned(),
        "'Yaru'".to_owned(),
    );
    let dconf = FakeDconf {
        live,
        ..FakeDconf::default()
    };
    let mut state = record(
        None,
        &[("gtk-theme", "'Adwaita-dark'")],
        &[("gtk-theme", "")],
    );
    let notes = apply_toolkit(&state_dir("mixed"), &mut state, &look(), &dconf);
    assert!(
        dconf
            .wrote_to("/org/gnome/desktop/interface/gtk-theme")
            .is_empty()
    );
    assert_eq!(dconf.wrote_to(COLOR_SCHEME_KEY).len(), 1);
    assert_eq!(dconf.wrote().len(), 5);
    assert!(
        notes
            .iter()
            .any(|note| note.contains("gtk-theme") && note.contains("outside `scoot look`")),
        "{notes:?}"
    );
}

#[test]
fn the_first_write_adopts_whatever_is_there() {
    // No record yet: the documented first-apply exception.
    let mut live = HashMap::new();
    live.insert(COLOR_SCHEME_KEY.to_owned(), PREFER_LIGHT.to_owned());
    let dconf = FakeDconf {
        live,
        ..FakeDconf::default()
    };
    let mut state = record(None, &[], &[]);
    let notes = apply_toolkit(&state_dir("adopt"), &mut state, &look(), &dconf);
    assert_eq!(dconf.wrote_to(COLOR_SCHEME_KEY).len(), 1);
    assert_eq!(
        state.dconf_wrote.get("color-scheme").map(String::as_str),
        Some(PREFER_DARK)
    );
    assert!(
        notes.iter().any(|note| note.contains("adopting")),
        "{notes:?}"
    );
}

#[test]
fn toolkit_off_writes_nothing() {
    let dconf = FakeDconf::default();
    let mut state = record(Some(false), &[], &[]);
    let notes = apply_toolkit(&state_dir("off"), &mut state, &look(), &dconf);
    assert!(dconf.wrote().is_empty());
    assert!(state.dconf_wrote.is_empty());
    assert!(notes.iter().any(|note| note.contains("off")), "{notes:?}");
}

#[test]
fn a_missing_dconf_is_a_note_not_a_refusal() {
    let dconf = FakeDconf {
        missing: true,
        ..FakeDconf::default()
    };
    let mut state = record(None, &[], &[]);
    let notes = apply_toolkit(&state_dir("missing"), &mut state, &look(), &dconf);
    assert!(dconf.wrote().is_empty());
    assert!(
        notes.iter().any(|note| note.contains("no `dconf`")),
        "{notes:?}"
    );
}

#[test]
fn a_failed_write_is_a_note_and_keeps_no_record() {
    let dconf = FakeDconf {
        fail_write: true,
        ..FakeDconf::default()
    };
    let mut state = record(None, &[], &[]);
    let notes = apply_toolkit(&state_dir("fail"), &mut state, &look(), &dconf);
    assert!(state.dconf_wrote.is_empty());
    assert!(
        notes.iter().any(|note| note.contains("not written")),
        "{notes:?}"
    );
}

#[test]
fn an_unreadable_key_is_left_alone_and_saves_no_original() {
    // Unreadable is not unset: nothing is written, and `""`
    // ("was unset") is never recorded for a key never seen.
    let dconf = FakeDconf {
        unreadable: true,
        ..FakeDconf::default()
    };
    let mut state = record(None, &[], &[]);
    let notes = apply_toolkit(&state_dir("unreadable"), &mut state, &look(), &dconf);
    assert!(dconf.wrote().is_empty());
    assert!(state.dconf_wrote.is_empty());
    assert!(state.dconf_orig.is_empty());
    assert!(
        notes.iter().any(|note| note.contains("unreadable")),
        "{notes:?}"
    );
}

#[test]
fn off_restores_every_owned_key_and_clears_the_record() {
    let mut live = HashMap::new();
    live.insert(COLOR_SCHEME_KEY.to_owned(), PREFER_DARK.to_owned());
    live.insert(
        "/org/gnome/desktop/interface/gtk-theme".to_owned(),
        "'Adwaita-dark'".to_owned(),
    );
    let dconf = FakeDconf {
        live,
        ..FakeDconf::default()
    };
    let mut state = record(
        None,
        &[
            ("color-scheme", PREFER_DARK),
            ("gtk-theme", "'Adwaita-dark'"),
        ],
        &[("color-scheme", PREFER_LIGHT), ("gtk-theme", "")],
    );
    let notes = toolkit_off(&state_dir("off-restore"), &mut state, &dconf);
    assert_eq!(
        dconf.wrote_to(COLOR_SCHEME_KEY),
        vec![PREFER_LIGHT.to_owned()]
    );
    assert!(
        dconf.wrote().contains(&(
            "reset /org/gnome/desktop/interface/gtk-theme".to_owned(),
            String::new()
        )),
        "{:?}",
        dconf.wrote()
    );
    assert!(state.dconf_wrote.is_empty());
    assert!(state.dconf_orig.is_empty());
    assert!(
        !notes.iter().any(|note| note.contains("left alone")),
        "{notes:?}"
    );
}

#[test]
fn off_leaves_a_key_someone_else_changed() {
    let mut live = HashMap::new();
    live.insert(COLOR_SCHEME_KEY.to_owned(), PREFER_LIGHT.to_owned());
    let dconf = FakeDconf {
        live,
        ..FakeDconf::default()
    };
    let mut state = record(
        None,
        &[("color-scheme", PREFER_DARK)],
        &[("color-scheme", "")],
    );
    let notes = toolkit_off(&state_dir("off-foreign"), &mut state, &dconf);
    assert!(dconf.wrote().is_empty());
    assert!(
        notes.iter().any(|note| note.contains("left alone")),
        "{notes:?}"
    );
    // Cleared anyway: the tool no longer owns the key.
    assert!(state.dconf_wrote.is_empty());
    assert!(state.dconf_orig.is_empty());
}

#[test]
fn off_writes_nothing_it_never_wrote() {
    // No record of a write (`apply --no-toolkit` first): `reset` must not
    // run against a key the tool never touched.
    let dconf = FakeDconf::default();
    let mut state = record(None, &[], &[]);
    let notes = toolkit_off(&state_dir("off-empty"), &mut state, &dconf);
    assert!(dconf.wrote().is_empty());
    assert!(
        notes.iter().any(|note| note.contains("written by")),
        "{notes:?}"
    );
}

#[test]
fn off_then_on_adopts_fresh_instead_of_crying_outside() {
    // After `off` the record holds no stale write while live holds the
    // original, so the next apply adopts (the documented first-write
    // path) instead of reporting "set outside `scoot look`" forever.
    // The fake backend's live value is static, so the off restores the
    // owned write and the apply adopts over the fake's unchanged live.
    let mut live = HashMap::new();
    live.insert(COLOR_SCHEME_KEY.to_owned(), PREFER_DARK.to_owned());
    let dconf = FakeDconf {
        live,
        ..FakeDconf::default()
    };
    let mut state = record(
        None,
        &[("color-scheme", PREFER_DARK)],
        &[("color-scheme", PREFER_LIGHT)],
    );
    toolkit_off(&state_dir("off-on"), &mut state, &dconf);
    assert!(state.dconf_wrote.is_empty());
    assert!(state.dconf_orig.is_empty());
    let notes = apply_toolkit(&state_dir("off-on"), &mut state, &look(), &dconf);
    // One restore write plus six fresh ones.
    assert_eq!(dconf.wrote_to(COLOR_SCHEME_KEY).len(), 2);
    assert_eq!(
        state.dconf_wrote.get("color-scheme").map(String::as_str),
        Some(PREFER_DARK)
    );
    assert!(
        notes.iter().any(|note| note.contains("adopting")),
        "{notes:?}"
    );
    assert!(
        !notes
            .iter()
            .any(|note| note.contains("outside `scoot look`")),
        "{notes:?}"
    );
}

#[test]
fn a_repeated_off_restores_nothing() {
    let mut live = HashMap::new();
    live.insert(COLOR_SCHEME_KEY.to_owned(), PREFER_LIGHT.to_owned());
    let dconf = FakeDconf {
        live,
        ..FakeDconf::default()
    };
    let mut state = record(
        None,
        &[("color-scheme", PREFER_DARK)],
        &[("color-scheme", "")],
    );
    toolkit_off(&state_dir("off-twice"), &mut state, &dconf);
    // Live now holds the restored original, and the record is clear: a
    // second `off` must not touch the key again.
    let dconf2 = FakeDconf {
        live: HashMap::from([(COLOR_SCHEME_KEY.to_owned(), PREFER_LIGHT.to_owned())]),
        ..FakeDconf::default()
    };
    let notes = toolkit_off(&state_dir("off-twice"), &mut state, &dconf2);
    assert!(dconf2.wrote().is_empty());
    assert!(
        notes.iter().any(|note| note.contains("written by")),
        "{notes:?}"
    );
}

#[test]
fn apply_from_a_private_state_touches_nothing() {
    struct Blind;
    impl Dconf for Blind {
        fn available(&self) -> bool {
            panic!("a private apply must not probe the backend");
        }
        fn read(&self, _key: &str) -> Result<Option<String>, String> {
            panic!("a private apply must not read the key");
        }
        fn write(&self, _key: &str, _value: &str) -> Result<(), String> {
            panic!("a private apply must not write the key");
        }
        fn reset(&self, _key: &str) -> Result<(), String> {
            panic!("a private apply must not reset the key");
        }
    }
    let dir = std::env::temp_dir().join(format!("scoot-look-apply-{}", std::process::id()));
    let mut state = record(None, &[], &[]);
    let notes = apply_toolkit(&dir, &mut state, &look(), &Blind);
    assert!(state.dconf_wrote.is_empty());
    assert!(state.dconf_orig.is_empty());
    assert!(
        notes.iter().any(|note| note.contains("private look state")),
        "{notes:?}"
    );
}

#[test]
fn off_from_a_private_state_touches_nothing() {
    // A headless/`--private-look` session's `toolkit off` never touches
    // the real keys: no read, no write, no reset — only the record clears.
    struct Blind;
    impl Dconf for Blind {
        fn available(&self) -> bool {
            panic!("a private off must not probe the backend");
        }
        fn read(&self, _key: &str) -> Result<Option<String>, String> {
            panic!("a private off must not read the key");
        }
        fn write(&self, _key: &str, _value: &str) -> Result<(), String> {
            panic!("a private off must not write the key");
        }
        fn reset(&self, _key: &str) -> Result<(), String> {
            panic!("a private off must not reset the key");
        }
    }
    let dir = std::env::temp_dir().join(format!("scoot-look-test-{}", std::process::id()));
    let mut state = record(
        None,
        &[("color-scheme", PREFER_DARK)],
        &[("color-scheme", "")],
    );
    let notes = toolkit_off(&dir, &mut state, &Blind);
    assert!(state.dconf_wrote.is_empty());
    assert!(state.dconf_orig.is_empty());
    assert!(
        notes.iter().any(|note| note.contains("private look state")),
        "{notes:?}"
    );
}

#[test]
fn records_round_trip_the_toolkit_half() {
    // Back-compat: a record written before the toolkit half reads with the
    // new fields absent, and a legacy single-key record folds into the
    // maps. A record carrying the maps keeps them and never writes the
    // legacy flat fields back.
    use scoot_look::state::{RecordRead, read_record};
    let dir = std::env::temp_dir().join(format!("scoot-look-toolkit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    let old = r#"{"name":"moonrise","rendered":"/r","generation":1}"#;
    std::fs::write(dir.join("look.json"), old).expect("an old record");
    let record = match read_record(&dir) {
        RecordRead::Present(record) => record,
        other => panic!("an old record reads: {other:?}"),
    };
    assert_eq!(record.toolkit, None);
    assert!(record.dconf_wrote.is_empty());
    assert!(toolkit_on(&record));
    let legacy = r#"{"name":"moonrise","rendered":"/r","generation":1,"color-scheme":"'prefer-dark'","color-scheme-orig":"'prefer-light'"}"#;
    std::fs::write(dir.join("look.json"), legacy).expect("a legacy record");
    let migrated = match read_record(&dir) {
        RecordRead::Present(record) => record,
        other => panic!("a legacy record reads: {other:?}"),
    };
    assert_eq!(
        migrated.dconf_wrote.get("color-scheme").map(String::as_str),
        Some(PREFER_DARK)
    );
    assert_eq!(
        migrated.dconf_orig.get("color-scheme").map(String::as_str),
        Some(PREFER_LIGHT)
    );
    let mut full = migrated;
    full.toolkit = Some(false);
    full.dconf_wrote
        .insert("gtk-theme".to_owned(), "'Adwaita-dark'".to_owned());
    scoot_look::state::write_record(&dir, &full).expect("a new record writes");
    let text = std::fs::read_to_string(dir.join("look.json")).expect("readable");
    let value: HashMap<String, serde_json::Value> = serde_json::from_str(&text).expect("JSON");
    assert_eq!(value["toolkit"], serde_json::Value::Bool(false));
    assert!(
        !value.contains_key("color-scheme"),
        "legacy field written back"
    );
    assert_eq!(
        value["dconf"]["color-scheme"],
        serde_json::Value::from(PREFER_DARK)
    );
    let back = match read_record(&dir) {
        RecordRead::Present(record) => record,
        other => panic!("a new record reads: {other:?}"),
    };
    assert_eq!(back, full);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn files_cover_gtk_qt_and_fonts() {
    // Six files: the two `@import` shims, the two inis, qt6ct through
    // the link's scheme, and the fontconfig default.
    let files = wanted_files(&look(), &link(&state_dir("files"))).expect("files");
    let rels: Vec<&str> = files.iter().map(|file| file.rel).collect();
    assert_eq!(
        rels,
        vec![
            "gtk-3.0/gtk.css",
            "gtk-4.0/gtk.css",
            "gtk-3.0/settings.ini",
            "gtk-4.0/settings.ini",
            "qt6ct/qt6ct.conf",
            "fontconfig/conf.d/10-scoot-look.conf",
        ]
    );
    for file in &files {
        assert!(
            file.contents
                .lines()
                .take(5)
                .any(|line| line.contains(MANAGED_MARK)),
            "{} carries no marker",
            file.rel
        );
    }
    let css = &files[0].contents;
    assert!(css.contains("@import url(\"file://"), "no @import:\n{css}");
    assert!(
        css.contains("/gtk/gtk-3.0.css\""),
        "not through the link:\n{css}"
    );
    let css4 = &files[1].contents;
    assert!(
        css4.contains("/gtk/gtk-4.0.css\""),
        "not through the link:\n{css4}"
    );
    let qt = &files[4].contents;
    assert!(qt.contains("style=Adwaita-Dark"), "wrong style:\n{qt}");
    assert!(
        qt.contains("/qt/scoot-look.conf"),
        "not through the link:\n{qt}"
    );
}

#[test]
fn a_backslash_in_the_state_path_refuses_the_files() {
    // A backslash is an escape in the CSS `@import` string while staying
    // literal in the qt6ct value: refuse it rather than writing a file
    // that points nowhere.
    let state = state_dir("backslash");
    let link = state.join("we\\ird");
    let error = wanted_files(&look(), &link).expect_err("a backslash refuses");
    assert!(error.contains("not a plain path"), "{error}");
}

#[test]
fn absent_files_are_written_and_identical_ones_kept() {
    let home = super::super::catalog::tests::scratch("files-write");
    let state = state_dir("files-write");
    let state_record = record(None, &[], &[]);
    let notes = apply_toolkit_files(
        &state,
        &state_record,
        &look(),
        Some(home.path()),
        &link(&state),
    );
    assert_eq!(
        notes
            .iter()
            .filter(|note| note.starts_with("wrote "))
            .count(),
        6
    );
    for rel in [
        "gtk-3.0/gtk.css",
        "gtk-4.0/gtk.css",
        "gtk-3.0/settings.ini",
        "gtk-4.0/settings.ini",
        "qt6ct/qt6ct.conf",
        "fontconfig/conf.d/10-scoot-look.conf",
    ] {
        assert!(home.path().join(rel).is_file(), "{rel} missing");
    }
    // A second run changes nothing and says nothing.
    let notes = apply_toolkit_files(
        &state,
        &state_record,
        &look(),
        Some(home.path()),
        &link(&state),
    );
    assert!(notes.is_empty(), "{notes:?}");
}

#[test]
fn marked_files_update_and_foreign_files_stay() {
    let home = super::super::catalog::tests::scratch("files-mixed");
    let state = state_dir("files-mixed");
    let state_record = record(None, &[], &[]);
    // A stale managed file updates; a hand-written one stays.
    std::fs::create_dir_all(home.path().join("gtk-3.0")).expect("a dir");
    std::fs::write(
        home.path().join("gtk-3.0/gtk.css"),
        format!("/* {MANAGED_MARK} (old) */\n@import url(\"file:///old/gtk.css\");\n"),
    )
    .expect("a stale shim");
    std::fs::create_dir_all(home.path().join("qt6ct")).expect("a dir");
    std::fs::write(
        home.path().join("qt6ct/qt6ct.conf"),
        "[Appearance]\nstyle=Fusion\n",
    )
    .expect("a hand config");
    let notes = apply_toolkit_files(
        &state,
        &state_record,
        &look(),
        Some(home.path()),
        &link(&state),
    );
    assert!(
        notes
            .iter()
            .any(|note| note.contains("updated gtk-3.0/gtk.css")),
        "{notes:?}"
    );
    assert!(
        notes
            .iter()
            .any(|note| note.contains("qt6ct/qt6ct.conf") && note.contains("left alone")),
        "{notes:?}"
    );
    let qt = std::fs::read_to_string(home.path().join("qt6ct/qt6ct.conf")).expect("readable");
    assert!(qt.contains("style=Fusion"), "a foreign file was touched");
}

#[test]
fn files_off_and_private_and_homeless_all_skip() {
    let home = super::super::catalog::tests::scratch("files-gates");
    let state = state_dir("files-gates");
    let off = record(Some(false), &[], &[]);
    let notes = apply_toolkit_files(&state, &off, &look(), Some(home.path()), &link(&state));
    assert!(notes.iter().any(|note| note.contains("off")), "{notes:?}");
    let private = std::env::temp_dir().join(format!("scoot-look-gates-{}", std::process::id()));
    let on = record(None, &[], &[]);
    let notes = apply_toolkit_files(&private, &on, &look(), Some(home.path()), &link(&private));
    assert!(
        notes.iter().any(|note| note.contains("private look state")),
        "{notes:?}"
    );
    let notes = apply_toolkit_files(&state, &on, &look(), None, &link(&state));
    assert!(
        notes.iter().any(|note| note.contains("no home")),
        "{notes:?}"
    );
    assert!(
        std::fs::read_dir(home.path())
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true),
        "a gated run wrote files"
    );
}
