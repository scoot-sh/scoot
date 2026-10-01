//! The payload target's seed corpus and every past finding, replayed on the
//! stable toolchain through the target's own check.

use std::path::{Path, PathBuf};

fn fuzz_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz")
}

/// Every file in `dir` but dotfiles (a `.gitkeep`), sorted.
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
fn the_payload_corpus_and_every_past_finding_replay_cleanly() {
    let corpus = files(&fuzz_dir().join("corpus").join("payload"));
    let regressions = files(&fuzz_dir().join("regressions").join("payload"));
    assert!(corpus.len() >= 6, "{} seed files", corpus.len());
    for path in corpus.iter().chain(&regressions) {
        let data = std::fs::read(path).unwrap();
        eprintln!("{} ({} bytes)", path.display(), data.len());
        super::payload(&data);
    }
}

#[test]
fn the_check_runs_on_what_it_is_given() {
    // Nothing, not UTF-8, and a hostile one: no panic.
    super::payload(b"");
    super::payload(&[0xff, 0xfe, 0x00, b'\n']);
    super::payload(&[b'['; 5000]);
    super::payload(&vec![b'x'; 10_000]);
}
