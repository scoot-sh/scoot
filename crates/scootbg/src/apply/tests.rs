use std::path::PathBuf;

use super::{Reply, classify, record_clear};
use crate::section::Section;
use crate::state::format::{self, Pick};
use crate::state::{self, Profile};

/// A scratch directory for one test, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "sbg-apply-{}-{tag}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scoot() -> Profile {
    Profile::parse("scoot").unwrap()
}

fn empty() -> Section {
    Section::parse(b"{}").unwrap()
}

#[test]
fn replies_are_classified() {
    assert_eq!(classify("{\"type\":\"ok\"}\n"), Reply::Ok);
    assert_eq!(
        classify(r#"{"type":"version","protocol":1,"version":"0.1.0"}"#),
        Reply::Version("0.1.0".into())
    );
    assert_eq!(
        classify(r#"{"type":"error","message":"unknown request `apply-config`"}"#),
        Reply::Error("unknown request `apply-config`".into())
    );
    assert_eq!(
        classify(r#"{"type":"error"}"#),
        Reply::Error("(no message)".into())
    );
    for other in [
        "",
        "garbage",
        r#"{"type":"version"}"#,
        r#"{"type":"outputs"}"#,
        r#"[1]"#,
    ] {
        assert_eq!(classify(other), Reply::Other(other.into()), "{other}");
    }
}

/// With no daemon, an empty section is recorded straight into the state
/// file: every choice cleared, the fingerprint set, the profile's other
/// lines gone; and recorded once only.
#[test]
fn an_empty_section_is_recorded_without_a_daemon() {
    let scratch = Scratch::new("clear");
    let dir = scratch.0.join("scootbg");
    let file = dir.join("scoot");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        &file,
        "scootbg-state 1\nprofile scoot\nfingerprint old\nall color #102030\n\
         output DP-1 color #405060\n",
    )
    .unwrap();
    record_clear(Some(&dir), &scoot(), &empty()).unwrap();
    let parsed = state::load(&file).unwrap().unwrap();
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    assert_eq!(
        parsed.record.fingerprint.as_deref(),
        Some(empty().fingerprint().as_str())
    );
    assert_eq!(parsed.record.all, Some(Pick::Clear));
    assert!(parsed.record.named.is_empty());
    assert_eq!(parsed.record.profile.as_deref(), Some("scoot"));

    // Recorded already: not written again (the file's time stays, and a
    // hand edit after it survives).
    let edited = format!(
        "{}output DP-9 color #000000\n",
        std::fs::read_to_string(&file).unwrap()
    );
    std::fs::write(&file, &edited).unwrap();
    record_clear(Some(&dir), &scoot(), &empty()).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), edited);

    // `command` alone is the same section.
    let command = Section::parse(br#"{"command":"/bin/scootbg"}"#).unwrap();
    record_clear(Some(&dir), &scoot(), &command).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), edited);
}

#[test]
fn an_empty_section_makes_a_missing_file() {
    let scratch = Scratch::new("new");
    let dir = scratch.0.join("state").join("scootbg");
    record_clear(Some(&dir), &scoot(), &empty()).unwrap();
    let text = std::fs::read_to_string(dir.join("scoot")).unwrap();
    let mut want = String::new();
    format::encode(
        &mut want,
        "scoot",
        Some(&empty().fingerprint()),
        Some(&None),
        &[],
    );
    assert_eq!(text, want);
    // Nowhere to keep state: nothing to do, and no error.
    record_clear(None, &scoot(), &empty()).unwrap();
}

/// A newer scootbg's file, or one that cannot be read, is never written
/// over: an error, the file untouched.
#[test]
fn a_file_it_cannot_use_is_left_alone() {
    let scratch = Scratch::new("left");
    let dir = scratch.0.join("scootbg");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("scoot");
    let newer = "scootbg-state 99\nwhatever\n";
    std::fs::write(&file, newer).unwrap();
    let error = record_clear(Some(&dir), &scoot(), &empty()).unwrap_err();
    assert!(error.contains("newer"), "{error}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), newer);

    // A directory in its place stands for an unreadable file (as root,
    // permission bits would not bind).
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    let error = record_clear(Some(&dir), &scoot(), &empty()).unwrap_err();
    assert!(error.contains("cannot read"), "{error}");
    assert!(file.is_dir());
}
