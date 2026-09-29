//! The fuzz targets' seed corpus and every past finding, replayed on the
//! stable toolchain through the targets' own checks.

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

/// Replays `target`'s seed corpus (at least `seeds` files, so a moved or
/// emptied corpus fails rather than passes) and its regressions.
fn replay(target: &str, seeds: usize, check: fn(&[u8])) {
    let corpus = files(&fuzz_dir().join("corpus").join(target));
    let regressions = files(&fuzz_dir().join("regressions").join(target));
    assert!(
        corpus.len() >= seeds,
        "{target}: {} seed files",
        corpus.len()
    );
    for path in corpus.iter().chain(&regressions) {
        let data = std::fs::read(path).unwrap();
        eprintln!("{} ({} bytes)", path.display(), data.len());
        check(&data);
    }
}

#[test]
fn the_format_corpus_and_every_past_finding_replay_cleanly() {
    replay("format", 3, super::format);
}

#[test]
fn the_tzif_corpus_and_every_past_finding_replay_cleanly() {
    replay("tzif", 13, super::tzif);
}

/// The checks reject what they are meant to: an input no target should
/// accept is not waved through. (The fuzz crate's targets are these
/// functions, so this is also the targets' own test.)
#[test]
fn the_checks_run_on_what_they_are_given() {
    // Empty, and not UTF-8: nothing to check, no panic.
    super::format(b"");
    super::format(b"\x00\xff");
    super::tzif(b"");
    // A format using every specifier kind, and a POSIX rule with DST.
    super::format(b"\x07%a %d %b %H:%M:%S %Z %z %%");
    super::tzif(b"EST5EDT,M3.2.0,M11.1.0");
    super::tzif(include_bytes!("../fixtures/Europe_Dublin.fat.tzif"));
}
