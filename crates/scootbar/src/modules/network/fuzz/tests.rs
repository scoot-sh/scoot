//! The network target's seed corpus and every past finding, replayed on the
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
fn the_network_corpus_and_every_past_finding_replay_cleanly() {
    let corpus = files(&fuzz_dir().join("corpus").join("network"));
    let regressions = files(&fuzz_dir().join("regressions").join("network"));
    assert!(corpus.len() >= 6, "network: {} seed files", corpus.len());
    for path in corpus.iter().chain(&regressions) {
        let data = std::fs::read(path).unwrap();
        eprintln!("{} ({} bytes)", path.display(), data.len());
        super::network(&data);
    }
}

/// The check runs on what it is given: empty, hostile and real-shaped
/// inputs are refused or accepted without a panic.
#[test]
fn the_check_runs_on_what_it_is_given() {
    super::network(b"");
    super::network(b"\x00\xff\x10\x00L\xff\xff\xff\xff");
    // A NEWLINK for eth0 and a scan BSS for Wimbly, framed as the fake
    // speaks them (family, pad, then the native-endian type).
    super::network(&[
        40, 0, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 2, 0, 0, 0, 1, 0, 0, 0, 8, 0,
        3, 0, b'e', b't', b'h', b'0', 0, 0, 0, 0,
    ]);
}
