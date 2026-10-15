//! Toolkit tests: the D11 rules against a scripted settings backend.

use std::cell::RefCell;
use std::collections::HashMap;

use scoot_look::state::Record;

use super::{
    COLOR_SCHEME_KEY, Dconf, PREFER_DARK, PREFER_LIGHT, apply_toolkit, toolkit_off, toolkit_on,
};

fn record(toolkit: Option<bool>, color_scheme: Option<&str>) -> Record {
    record_orig(toolkit, color_scheme, None)
}

fn record_orig(toolkit: Option<bool>, color_scheme: Option<&str>, orig: Option<&str>) -> Record {
    Record {
        name: "moonrise".to_owned(),
        rendered: "/r/moonrise".to_owned(),
        generation: 1,
        chosen_by: Some("apply".to_owned()),
        declared_default: None,
        toolkit,
        color_scheme: color_scheme.map(str::to_owned),
        color_scheme_orig: orig.map(str::to_owned),
    }
}

fn state_dir() -> std::path::PathBuf {
    // A non-private scratch dir: the leaf must not start with the private
    // prefix (`scoot-look-`), and SCOOT_LOOK_STATE is not pointed at it by
    // the tests.
    std::env::temp_dir().join(format!("scoot-toolkit-state-{}", std::process::id()))
}

#[derive(Default)]
struct FakeDconf {
    live: Option<String>,
    writes: RefCell<Vec<(String, String)>>,
    fail_write: bool,
    missing: bool,
    unreadable: bool,
}

impl FakeDconf {
    fn wrote(&self) -> Vec<(String, String)> {
        self.writes.borrow().clone()
    }
}

impl Dconf for FakeDconf {
    fn available(&self) -> bool {
        !self.missing
    }

    fn read(&self, _key: &str) -> Result<Option<String>, String> {
        if self.unreadable {
            return Err("dconf is broken".to_owned());
        }
        Ok(self.live.clone())
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
    assert!(toolkit_on(&record(None, None)));
    assert!(toolkit_on(&record(Some(true), None)));
    assert!(!toolkit_on(&record(Some(false), None)));
}

#[test]
fn an_unset_key_is_written_and_recorded() {
    let dconf = FakeDconf::default();
    let mut state = record(None, None);
    let notes = apply_toolkit(&state_dir(), &mut state, true, &dconf);
    assert_eq!(
        dconf.wrote(),
        vec![(COLOR_SCHEME_KEY.to_owned(), PREFER_DARK.to_owned())]
    );
    assert_eq!(state.color_scheme.as_deref(), Some(PREFER_DARK));
    assert!(
        notes.iter().any(|note| note.contains(PREFER_DARK)),
        "{notes:?}"
    );
}

#[test]
fn a_light_look_writes_prefer_light() {
    let dconf = FakeDconf::default();
    let mut state = record(None, None);
    apply_toolkit(&state_dir(), &mut state, false, &dconf);
    assert_eq!(
        dconf.wrote(),
        vec![(COLOR_SCHEME_KEY.to_owned(), PREFER_LIGHT.to_owned())]
    );
}

#[test]
fn what_the_tool_wrote_is_owned_and_rewritten() {
    let dconf = FakeDconf {
        live: Some(PREFER_DARK.to_owned()),
        ..FakeDconf::default()
    };
    let mut state = record(None, Some(PREFER_DARK));
    let notes = apply_toolkit(&state_dir(), &mut state, true, &dconf);
    assert_eq!(dconf.wrote().len(), 1);
    assert!(
        notes.iter().any(|note| note.contains("set to")),
        "{notes:?}"
    );
}

#[test]
fn a_key_someone_else_changed_is_left_alone_and_named() {
    let dconf = FakeDconf {
        live: Some(PREFER_LIGHT.to_owned()),
        ..FakeDconf::default()
    };
    let mut state = record(None, Some(PREFER_DARK));
    let notes = apply_toolkit(&state_dir(), &mut state, true, &dconf);
    assert!(dconf.wrote().is_empty());
    assert_eq!(state.color_scheme.as_deref(), Some(PREFER_DARK));
    assert!(
        notes
            .iter()
            .any(|note| note.contains("outside `scoot look`")),
        "{notes:?}"
    );
}

#[test]
fn the_first_write_adopts_whatever_is_there() {
    // No record yet: the documented first-apply exception.
    let dconf = FakeDconf {
        live: Some(PREFER_LIGHT.to_owned()),
        ..FakeDconf::default()
    };
    let mut state = record(None, None);
    let notes = apply_toolkit(&state_dir(), &mut state, true, &dconf);
    assert_eq!(dconf.wrote().len(), 1);
    assert_eq!(state.color_scheme.as_deref(), Some(PREFER_DARK));
    assert!(
        notes.iter().any(|note| note.contains("adopting")),
        "{notes:?}"
    );
}

#[test]
fn toolkit_off_writes_nothing() {
    let dconf = FakeDconf::default();
    let mut state = record(Some(false), None);
    let notes = apply_toolkit(&state_dir(), &mut state, true, &dconf);
    assert!(dconf.wrote().is_empty());
    assert!(state.color_scheme.is_none());
    assert!(notes.iter().any(|note| note.contains("off")), "{notes:?}");
}

#[test]
fn a_missing_dconf_is_a_note_not_a_refusal() {
    let dconf = FakeDconf {
        missing: true,
        ..FakeDconf::default()
    };
    let mut state = record(None, None);
    let notes = apply_toolkit(&state_dir(), &mut state, true, &dconf);
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
    let mut state = record(None, None);
    let notes = apply_toolkit(&state_dir(), &mut state, true, &dconf);
    assert!(state.color_scheme.is_none());
    assert!(
        notes.iter().any(|note| note.contains("not written")),
        "{notes:?}"
    );
}

#[test]
fn an_unreadable_key_is_left_alone_and_saves_no_original() {
    // Unreadable is not unset: nothing is written, and `Some("")`
    // ("was unset") is never recorded for a key never seen.
    let dconf = FakeDconf {
        unreadable: true,
        ..FakeDconf::default()
    };
    let mut state = record(None, None);
    let notes = apply_toolkit(&state_dir(), &mut state, true, &dconf);
    assert!(dconf.wrote().is_empty());
    assert!(state.color_scheme.is_none());
    assert!(state.color_scheme_orig.is_none());
    assert!(
        notes.iter().any(|note| note.contains("unreadable")),
        "{notes:?}"
    );
}

#[test]
fn off_restores_what_is_still_live_and_clears_the_record() {
    let dconf = FakeDconf {
        live: Some(PREFER_DARK.to_owned()),
        ..FakeDconf::default()
    };
    let mut state = record_orig(None, Some(PREFER_DARK), Some(PREFER_LIGHT));
    let notes = toolkit_off(&state_dir(), &mut state, &dconf);
    assert_eq!(
        dconf.wrote(),
        vec![(COLOR_SCHEME_KEY.to_owned(), PREFER_LIGHT.to_owned())]
    );
    assert!(state.color_scheme.is_none());
    assert!(state.color_scheme_orig.is_none());
    assert!(
        !notes.iter().any(|note| note.contains("left alone")),
        "{notes:?}"
    );
}

#[test]
fn off_resets_a_key_that_was_unset() {
    let dconf = FakeDconf {
        live: Some(PREFER_DARK.to_owned()),
        ..FakeDconf::default()
    };
    let mut state = record_orig(None, Some(PREFER_DARK), Some(""));
    toolkit_off(&state_dir(), &mut state, &dconf);
    assert_eq!(
        dconf.wrote(),
        vec![(format!("reset {COLOR_SCHEME_KEY}"), String::new())]
    );
    assert!(state.color_scheme.is_none());
}

#[test]
fn off_leaves_a_key_someone_else_changed() {
    let dconf = FakeDconf {
        live: Some(PREFER_LIGHT.to_owned()),
        ..FakeDconf::default()
    };
    let mut state = record_orig(None, Some(PREFER_DARK), Some(""));
    let notes = toolkit_off(&state_dir(), &mut state, &dconf);
    assert!(dconf.wrote().is_empty());
    assert!(
        notes.iter().any(|note| note.contains("left alone")),
        "{notes:?}"
    );
    // Cleared anyway: the tool no longer owns the key.
    assert!(state.color_scheme.is_none());
    assert!(state.color_scheme_orig.is_none());
}

#[test]
fn off_writes_nothing_it_never_wrote() {
    // No record of a write (`apply --no-toolkit` first): `reset` must not
    // run against a key the tool never touched.
    let dconf = FakeDconf::default();
    let mut state = record(None, None);
    let notes = toolkit_off(&state_dir(), &mut state, &dconf);
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
    let dconf = FakeDconf {
        live: Some(PREFER_DARK.to_owned()),
        ..FakeDconf::default()
    };
    let mut state = record_orig(None, Some(PREFER_DARK), Some(PREFER_LIGHT));
    toolkit_off(&state_dir(), &mut state, &dconf);
    assert!(state.color_scheme.is_none());
    assert!(state.color_scheme_orig.is_none());
    let notes = apply_toolkit(&state_dir(), &mut state, true, &dconf);
    assert_eq!(dconf.wrote().len(), 2);
    assert_eq!(state.color_scheme.as_deref(), Some(PREFER_DARK));
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
    let dconf = FakeDconf {
        live: Some(PREFER_LIGHT.to_owned()),
        ..FakeDconf::default()
    };
    let mut state = record_orig(None, Some(PREFER_DARK), Some(""));
    toolkit_off(&state_dir(), &mut state, &dconf);
    // Live now holds the restored original, and the record is clear: a
    // second `off` must not touch the key again.
    let dconf2 = FakeDconf {
        live: Some(PREFER_LIGHT.to_owned()),
        ..FakeDconf::default()
    };
    let notes = toolkit_off(&state_dir(), &mut state, &dconf2);
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
    let mut state = record(None, None);
    let notes = apply_toolkit(&dir, &mut state, true, &Blind);
    assert!(state.color_scheme.is_none());
    assert!(state.color_scheme_orig.is_none());
    assert!(
        notes.iter().any(|note| note.contains("private look state")),
        "{notes:?}"
    );
}

#[test]
fn off_from_a_private_state_touches_nothing() {
    // A headless/`--private-look` session's `toolkit off` never touches
    // the real key: no read, no write, no reset — only the record clears.
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
    let mut state = record_orig(None, Some(PREFER_DARK), Some(""));
    let notes = toolkit_off(&dir, &mut state, &Blind);
    assert!(state.color_scheme.is_none());
    assert!(state.color_scheme_orig.is_none());
    assert!(
        notes.iter().any(|note| note.contains("private look state")),
        "{notes:?}"
    );
}

#[test]
fn records_round_trip_the_toolkit_half() {
    // Back-compat: a record written before the toolkit half reads with the
    // new fields absent, and a record carrying them keeps them.
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
    assert_eq!(record.color_scheme, None);
    assert!(toolkit_on(&record));
    let mut full = record;
    full.toolkit = Some(false);
    full.color_scheme = Some(PREFER_DARK.to_owned());
    scoot_look::state::write_record(&dir, &full).expect("a new record writes");
    let text = std::fs::read_to_string(dir.join("look.json")).expect("readable");
    let value: HashMap<String, serde_json::Value> = serde_json::from_str(&text).expect("JSON");
    assert_eq!(value["toolkit"], serde_json::Value::Bool(false));
    assert_eq!(value["color-scheme"], serde_json::Value::from(PREFER_DARK));
    let back = match read_record(&dir) {
        RecordRead::Present(record) => record,
        other => panic!("a new record reads: {other:?}"),
    };
    assert_eq!(back, full);
    std::fs::remove_dir_all(&dir).ok();
}
