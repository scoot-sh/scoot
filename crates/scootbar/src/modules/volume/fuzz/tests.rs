//! The volume target's seed corpus and every past finding, replayed on the
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
fn the_volume_corpus_and_every_past_finding_replay_cleanly() {
    let corpus = files(&fuzz_dir().join("corpus").join("volume"));
    let regressions = files(&fuzz_dir().join("regressions").join("volume"));
    assert!(corpus.len() >= 8, "volume: {} seed files", corpus.len());
    for path in corpus.iter().chain(&regressions) {
        let data = std::fs::read(path).unwrap();
        eprintln!("{} ({} bytes)", path.display(), data.len());
        super::volume(&data);
    }
}

/// The check runs on what it is given: empty, hostile and real-shaped
/// inputs are refused or accepted without a panic.
#[test]
fn the_check_runs_on_what_it_is_given() {
    super::volume(b"");
    super::volume(b"\x00\xff\x10\x00L\xff\xff\xff\xff");
    // A framed AUTH reply, a subscribe event and a server-info reply in
    // the shapes the fake server speaks.
    super::volume(&[
        0, 0, 0, 15, 255, 255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, b'L', 0, 0, 0, 2,
        b'L', 0, 0, 0, 0, b'L', 0, 0, 0, 35,
    ]);
    super::volume(&[
        0, 0, 0, 20, 255, 255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, b'L', 0, 0, 0, 66,
        b'L', 255, 255, 255, 255, b'L', 0, 0, 0, 16, b'L', 0, 0, 0, 0,
    ]);
}
