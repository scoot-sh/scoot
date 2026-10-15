//! Toolkit tests: the D11 rules against a scripted settings backend.

use std::cell::RefCell;
use std::collections::HashMap;

use scoot_look::state::Record;

use super::{COLOR_SCHEME_KEY, Dconf, PREFER_DARK, PREFER_LIGHT, apply_toolkit, toolkit_on};

fn record(toolkit: Option<bool>, color_scheme: Option<&str>) -> Record {
    Record {
        name: "moonrise".to_owned(),
        rendered: "/r/moonrise".to_owned(),
        generation: 1,
        chosen_by: Some("apply".to_owned()),
        declared_default: None,
        toolkit,
        color_scheme: color_scheme.map(str::to_owned),
        color_scheme_orig: None,
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

    fn read(&self, _key: &str) -> Option<String> {
        self.live.clone()
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
