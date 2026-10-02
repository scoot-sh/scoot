//! The dbus target's seed corpus and every past finding, replayed on the
//! stable toolchain through the target's own check.

use std::path::{Path, PathBuf};

fn fuzz_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz")
}

/// Every file in `dir` but dotfiles (a `.gitkeep`), sorted, so a failure
/// names the same one each run.
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let hidden = path
                .file_name()
                .is_some_and(|name| name.as_encoded_bytes().starts_with(b"."));
            path.is_file() && !hidden
        })
        .collect();
    files.sort();
    files
}

#[test]
fn the_dbus_corpus_and_every_past_finding_replay_cleanly() {
    let corpus = files(&fuzz_dir().join("corpus").join("dbus"));
    let regressions = files(&fuzz_dir().join("regressions").join("dbus"));
    assert!(corpus.len() >= 8, "dbus: {} seed files", corpus.len());
    for path in corpus.iter().chain(&regressions) {
        let data = std::fs::read(path).unwrap();
        eprintln!("{} ({} bytes)", path.display(), data.len());
        super::dbus(&data);
    }
}

/// The check runs on what it is given: empty, hostile and real-shaped
/// inputs are refused or accepted without a panic.
#[test]
fn the_check_runs_on_what_it_is_given() {
    use super::super::proto::Writer;
    super::dbus(b"");
    super::dbus(b"\x00\xff\x10\x00L\xff\xff\xff\xff");
    super::dbus(b"l\x01\x00\x01\xff\xff\xff\xff\x01\x00\x00\x00");
    // A `Hello` reply in the shape the fake bus speaks, whole and cut.
    let mut writer = Writer::new();
    writer.begin_return(2, 1, "s");
    writer.str(":1.7");
    let mut hello = writer.finish().unwrap();
    super::dbus(&hello);
    hello.truncate(hello.len() / 2);
    super::dbus(&hello);
    // A `NameOwnerChanged` signal, and a pixmap property.
    let mut signal = Writer::new();
    signal.begin_signal(9, "/org/freedesktop/DBus", "org.freedesktop.DBus", "NameOwnerChanged", "sss");
    signal.str(":1.7");
    signal.str("");
    signal.str(":1.7");
    let body = signal.finish().unwrap();
    super::dbus(&body);
}
