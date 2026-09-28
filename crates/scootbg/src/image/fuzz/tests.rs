//! The fuzz target's corpus and crash files, replayed on the stable
//! toolchain, and the fuzz crate's lockfile kept to the daemon's versions.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::{HEADER, side, whole_path};
use crate::image::samples;

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

/// The committed seed corpus and every crash the fuzzer has found
/// (`fuzz/regressions/whole/`, each fixed), through the same entry point
/// the fuzz target calls: a panic here is the finding coming back.
#[test]
fn the_corpus_and_every_past_crash_replay_cleanly() {
    let corpus = files(&fuzz_dir().join("corpus/whole"));
    let regressions = files(&fuzz_dir().join("regressions/whole"));
    // Not vacuous: a moved or emptied corpus fails rather than passes.
    assert!(corpus.len() >= 10, "only {} seed files", corpus.len());
    for path in corpus.iter().chain(&regressions) {
        let data = std::fs::read(path).unwrap();
        eprintln!("{} ({} bytes)", path.display(), data.len());
        whole_path(&data);
    }
}

#[test]
fn an_input_shorter_than_the_header_does_nothing() {
    for len in 0..=HEADER {
        whole_path(&vec![0xff; len]);
    }
}

#[test]
fn the_top_side_values_are_sizes_only_a_configure_could_ask_for() {
    assert_eq!(side(0), 0);
    assert_eq!(side(1), 1);
    assert_eq!(side(65532), 65532);
    assert_eq!(side(0xfffd), 23_171);
    assert_eq!(side(0xfffe), 1 << 20);
    assert_eq!(side(0xffff), u32::MAX);
    // Just past `wl_shm`'s `int32` as a square, and 23170 just inside.
    assert!(23_171_u64 * 23_171 * 4 > i32::MAX as u64);
    assert!(23_170_u64 * 23_170 * 4 <= i32::MAX as u64);
}

/// Every mode, filter and orientation over a real image at sizes around
/// the source's, with `wl_shm`-refused sizes among them: the entry point
/// itself asserts what it draws, so this is its own check, not only the
/// corpus's.
#[test]
fn every_look_draws_or_refuses_without_a_panic() {
    let sizes: [[u16; 6]; 3] = [
        [1, 1, 31, 1, 0xfffd, 0xfffd],
        [32, 16, 1, 17, 7, 0],
        [97, 3, 3, 97, 0xffff, 2],
    ];
    for mode in 0..5_u8 {
        for filter in 0..4_u8 {
            for orientation in 0..9_u8 {
                for set in &sizes {
                    let mut data = vec![mode, filter, 1, 2, 3, orientation, 2];
                    for side in set {
                        data.extend_from_slice(&side.to_le_bytes());
                    }
                    assert_eq!(data.len(), HEADER);
                    data.extend_from_slice(samples::QUADRANTS_JPEG);
                    whole_path(&data);
                }
            }
        }
    }
}

/// `name` → every version of it, for the packages of a `Cargo.lock`.
fn locked(lockfile: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut packages: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut name = None;
    for line in lockfile.lines() {
        let quoted = |key: &str| {
            line.strip_prefix(key)
                .and_then(|rest| rest.strip_prefix(" = \""))
                .and_then(|rest| rest.strip_suffix('"'))
                .map(str::to_owned)
        };
        if line == "[[package]]" {
            name = None;
        } else if let Some(found) = quoted("name") {
            name = Some(found);
        } else if let (Some(version), Some(name)) = (quoted("version"), name.take()) {
            packages.entry(name).or_default().insert(version);
        }
    }
    packages
}

/// The fuzz crate has its own lockfile (it is not in the workspace), so
/// nothing makes it follow a dependency bump in the workspace's: this
/// does. Every version of a package both lock (the decoders, the scaler
/// and what they pull in) must be one the workspace locks, or the fuzzer
/// tests code the daemon does not run. On a failure, refresh it from the workspace's:
/// `cp Cargo.lock crates/scootbg/fuzz/Cargo.lock`, then `cargo update
/// --workspace` in `crates/scootbg/fuzz` (see its README).
#[test]
fn the_fuzz_lockfile_matches_the_workspace() {
    let read = |path: &Path| {
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    };
    let workspace = locked(&read(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock"),
    ));
    let fuzz = locked(&read(&fuzz_dir().join("Cargo.lock")));
    for name in [
        "zune-jpeg",
        "zune-core",
        "png",
        "image-webp",
        "pic-scale-safe",
    ] {
        assert!(
            fuzz.contains_key(name),
            "{name} is not in the fuzz lockfile"
        );
    }
    let differ: Vec<_> = fuzz
        .iter()
        .filter_map(|(name, versions)| {
            let theirs = workspace.get(name)?;
            (!versions.is_subset(theirs))
                .then(|| format!("{name}: fuzz {versions:?}, workspace {theirs:?}"))
        })
        .collect();
    assert!(differ.is_empty(), "{differ:#?}");
}
