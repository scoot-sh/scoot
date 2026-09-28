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

/// One `[[package]]` of a `Cargo.lock`: its name, version, and its
/// dependencies as written (`"name"`, or `"name version"` when the lock
/// holds more than one version of it, maybe with a source after).
struct Locked {
    name: String,
    version: String,
    dependencies: Vec<String>,
}

fn parse_lock(lockfile: &str) -> Vec<Locked> {
    let mut packages: Vec<Locked> = Vec::new();
    let mut in_dependencies = false;
    for line in lockfile.lines() {
        let quoted = |key: &str| {
            line.strip_prefix(key)
                .and_then(|rest| rest.strip_prefix(" = \""))
                .and_then(|rest| rest.strip_suffix('"'))
                .map(str::to_owned)
        };
        if line == "[[package]]" {
            packages.push(Locked {
                name: String::new(),
                version: String::new(),
                dependencies: Vec::new(),
            });
            in_dependencies = false;
            continue;
        }
        let Some(package) = packages.last_mut() else {
            continue;
        };
        if line == "dependencies = [" {
            in_dependencies = true;
        } else if in_dependencies {
            if line == "]" {
                in_dependencies = false;
            } else if let Some(entry) = line
                .trim()
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix("\","))
            {
                package.dependencies.push(entry.to_owned());
            }
        } else if let Some(name) = quoted("name") {
            package.name = name;
        } else if let Some(version) = quoted("version") {
            package.version = version;
        }
    }
    packages
}

/// A package in a lock, by name and version.
type Node = (String, String);

/// Every package reachable from `root` through the lock's dependency
/// edges, `root` included, each with the packages it depends on. A
/// dependency written without a version is the lock's only package of
/// that name (Cargo writes the version whenever there are several).
fn reachable(lock: &[Locked], root: &str) -> BTreeMap<Node, BTreeSet<Node>> {
    let find = |entry: &str| -> &Locked {
        let mut words = entry.split(' ');
        let name = words.next().unwrap_or_default();
        let version = words.next();
        let mut found = lock
            .iter()
            .filter(|p| p.name == name && version.is_none_or(|v| p.version == v));
        let package = found
            .next()
            .unwrap_or_else(|| panic!("the lock has no package for {entry:?}"));
        assert!(found.next().is_none(), "{entry:?} is ambiguous in the lock");
        package
    };
    let node = |p: &Locked| (p.name.clone(), p.version.clone());
    let mut graph: BTreeMap<Node, BTreeSet<Node>> = BTreeMap::new();
    let mut queue = vec![find(root)];
    while let Some(package) = queue.pop() {
        if graph.contains_key(&node(package)) {
            continue;
        }
        let dependencies: Vec<&Locked> = package
            .dependencies
            .iter()
            .map(|entry| find(entry))
            .collect();
        graph.insert(
            node(package),
            dependencies.iter().map(|p| node(p)).collect(),
        );
        queue.extend(dependencies);
    }
    graph
}

/// The graphs scootbg and the fuzz crate each reach, from their locks.
fn graphs(
    fuzz_lock: &str,
) -> (
    BTreeMap<Node, BTreeSet<Node>>,
    BTreeMap<Node, BTreeSet<Node>>,
) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    let workspace =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    (
        reachable(&parse_lock(&workspace), "scootbg"),
        reachable(&parse_lock(fuzz_lock), "scootbg-fuzz"),
    )
}

/// Where the fuzz crate's graph departs from scootbg's: a package both
/// reach at a version scootbg does not use, or a package at the same
/// version whose dependency resolves to another version (`png` taking
/// the other `miniz_oxide`). Empty when the two agree.
fn drift(fuzz_lock: &str) -> Vec<String> {
    let (daemon, fuzz) = graphs(fuzz_lock);
    for name in [
        "zune-jpeg",
        "zune-core",
        "png",
        "image-webp",
        "pic-scale-safe",
        "scootbg-mem",
    ] {
        assert!(
            fuzz.keys().any(|(n, _)| n == name),
            "the fuzz crate does not reach {name}"
        );
    }
    let mut differ = Vec::new();
    for (node, dependencies) in &fuzz {
        let theirs: Vec<&String> = daemon
            .keys()
            .filter(|(n, _)| *n == node.0)
            .map(|(_, v)| v)
            .collect();
        if theirs.is_empty() {
            continue;
        }
        let Some(expected) = daemon.get(node) else {
            differ.push(format!("{}: fuzz {}, scootbg {theirs:?}", node.0, node.1));
            continue;
        };
        // Features may add or drop a dependency (`serde_derive`, `cc`'s
        // `jobserver`); what may not differ is the version a dependency
        // both have resolves to.
        for (name, version) in dependencies {
            if let Some((_, wanted)) = expected.iter().find(|(n, _)| n == name) {
                if wanted != version {
                    differ.push(format!(
                        "{} {}: fuzz takes {name} {version}, scootbg {wanted}",
                        node.0, node.1
                    ));
                }
            }
        }
    }
    differ
}

fn fuzz_lock() -> String {
    let path = fuzz_dir().join("Cargo.lock");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The fuzz crate has its own lockfile (it is not in the workspace), so
/// nothing makes it follow a dependency bump in the workspace's: this
/// does. It follows the dependency edges of both locks, from `scootbg` in
/// the workspace's and from `scootbg-fuzz` in its own, and every package
/// both reach must be at a version scootbg itself resolves to, not merely
/// one the workspace locks for someone else (it locks two `rustix` and
/// two `miniz_oxide`). Otherwise the fuzzer tests code the daemon does
/// not run. On a failure, refresh it from the workspace's: `cp Cargo.lock
/// crates/scootbg/fuzz/Cargo.lock`, then `cargo update --workspace` in
/// `crates/scootbg/fuzz` (see its README).
#[test]
fn the_fuzz_lockfile_matches_the_workspace() {
    let differ = drift(&fuzz_lock());
    assert!(differ.is_empty(), "{differ:#?}");
}

/// The check is not vacuous. The workspace locks two `miniz_oxide`
/// (scootbg reaches both: `png` takes 0.8.9 and `flate2` 0.9.1), so a
/// fuzz lock whose `png` took 0.9.1 would pass a check by name and
/// version alone; the edges catch it. So does a version of `rustix` the
/// workspace locks only for another crate (0.38).
#[test]
fn a_dependency_scootbg_does_not_resolve_to_is_caught() {
    let lock = fuzz_lock();
    let edge = "\"fdeflate\",\n \"flate2\",\n \"miniz_oxide 0.8.9\",";
    assert!(
        lock.contains(edge),
        "png's edges are not as this test expects"
    );
    let planted = lock.replace(edge, "\"fdeflate\",\n \"flate2\",\n \"miniz_oxide 0.9.1\",");
    let differ = drift(&planted);
    eprintln!("png planted: {differ:#?}");
    assert!(
        differ
            .iter()
            .any(|line| line.starts_with("png ") && line.contains("miniz_oxide")),
        "{differ:#?}"
    );

    let rustix = "name = \"rustix\"\nversion = \"1.1.4\"";
    assert!(lock.contains(rustix));
    let differ = drift(&lock.replace(rustix, "name = \"rustix\"\nversion = \"0.38.44\""));
    eprintln!("rustix planted: {differ:#?}");
    assert!(
        differ
            .iter()
            .any(|line| line.starts_with("rustix: fuzz 0.38.44")),
        "{differ:#?}"
    );
}
