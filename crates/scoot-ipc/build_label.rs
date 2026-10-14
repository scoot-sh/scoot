// The version a binary prints, labeled with where it was built from.
//
// Not part of the `scoot-ipc` library: the `scoot`, `scootbg` and
// `scootbar` build scripts each load this file (`#[path]`), and
// `tests/build_label.rs` tests it. It lives here because all three
// binaries depend on scoot-ipc, so it is in every tree that builds them.
//
// A build script calls `emit()`, which sets `SCOOT_BUILD_VERSION` for the
// crate it builds. The binary reads it with `env!("SCOOT_BUILD_VERSION")`
// for `--version` and for its IPC `version` reply.
//
// - A tagged release, built at its `vX.Y.Z` tag or from the release
//   tarball (which has no `.git`), prints the plain version: `0.2.0`.
// - A build from any other commit of a git checkout prints the last tag,
//   the commits since it, and the commit: `0.2.0+14.g615608c` (SemVer
//   build metadata, so it sorts the same as `0.2.0`). Before the first
//   tag exists, it prints the manifest version and the total commit count.
// - No `.git`, no `git` binary, or any git error prints the plain manifest
//   version. So does a tree whose enclosing repository is some other one
//   (a release tarball unpacked inside a packager's own git checkout, as an
//   AUR clone is): the label only describes this repository's history. Nix builds get the plain version this way, since their source
//   has no `.git`. The label never fails a build.
//
// Uncommitted changes are not marked: cargo cannot cheaply tell this script
// that the work tree changed, so a `.dirty` suffix would go stale.
//
// Cost: the label changes with every commit, so each binary crate
// recompiles once after a commit or checkout. Only the binary crates do,
// because the label is set in their build scripts, not in a library that
// everything depends on.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// What git says about the commit being built.
#[derive(Debug, PartialEq, Eq)]
pub enum GitState {
    /// `git describe` found a `vX.Y.Z` tag: its version, the commits since
    /// it, and the abbreviated commit.
    Tagged {
        tag_version: String,
        since: u64,
        sha: String,
    },
    /// No release tag reachable: the total commit count and the commit.
    Untagged { commits: u64, sha: String },
}

/// The version string for `pkg_version`, given what git said (`None`:
/// not a git checkout, or git failed).
pub fn label(pkg_version: &str, state: Option<&GitState>) -> String {
    match state {
        None => pkg_version.to_string(),
        Some(GitState::Tagged {
            tag_version,
            since: 0,
            ..
        }) if tag_version == pkg_version => pkg_version.to_string(),
        Some(GitState::Tagged {
            tag_version,
            since,
            sha,
        }) => format!("{tag_version}+{since}.g{sha}"),
        Some(GitState::Untagged { commits, sha }) => format!("{pkg_version}+{commits}.g{sha}"),
    }
}

/// Parses `git describe --tags --long` output such as `v0.2.0-14-g615608c`.
/// Anything else (a tag that is not `vX.Y.Z`, a malformed count or commit)
/// is `None`, so a stray tag never produces a garbled version.
pub fn parse_describe(describe: &str) -> Option<GitState> {
    let mut parts = describe.trim().rsplitn(3, '-');
    let sha = parts.next()?.strip_prefix('g')?;
    let since = parts.next()?.parse().ok()?;
    let tag_version = parts.next()?.strip_prefix('v')?;
    if !is_release_version(tag_version) || !is_hex(sha) {
        return None;
    }
    Some(GitState::Tagged {
        tag_version: tag_version.to_string(),
        since,
        sha: sha.to_string(),
    })
}

fn is_release_version(version: &str) -> bool {
    let fields: Vec<&str> = version.split('.').collect();
    fields.len() == 3
        && fields
            .iter()
            .all(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
}

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Asks git about the checkout holding `dir`, which must be the top of
/// that checkout (`root`, the workspace root): a tree nested inside some
/// other repository gets `None`. Returns the state plus the files and
/// directories whose change means the answer may have changed (only paths
/// that exist: cargo reruns a script whose watched path is missing on every
/// build).
pub fn git_state(dir: &Path, root: &Path) -> (Option<GitState>, Vec<PathBuf>) {
    let Some(dirs) = git(
        dir,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--show-toplevel",
            "--git-dir",
            "--git-common-dir",
        ],
    ) else {
        return (None, Vec::new());
    };
    let mut lines = dirs.lines();
    let (Some(toplevel), Some(git_dir), Some(common_dir)) =
        (lines.next(), lines.next(), lines.next())
    else {
        return (None, Vec::new());
    };
    let same = |a: &Path, b: &Path| match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same(Path::new(toplevel), root) {
        return (None, Vec::new());
    }
    let (git_dir, common_dir) = (PathBuf::from(git_dir), PathBuf::from(common_dir));
    // HEAD is per worktree; refs are shared. A commit moves a branch ref
    // (loose under refs/heads, or packed); a tag lands under refs/tags or
    // in packed-refs.
    let watch = [
        git_dir.join("HEAD"),
        common_dir.join("packed-refs"),
        common_dir.join("refs").join("heads"),
        common_dir.join("refs").join("tags"),
    ]
    .into_iter()
    .filter(|p| p.exists())
    .collect();

    let state = git(
        dir,
        &[
            "describe",
            "--tags",
            "--long",
            "--abbrev=7",
            "--match",
            "v[0-9]*",
            // A pre-release tag (`v0.3.0-rc1`) is not a release: skip past
            // it to the last real one.
            "--exclude",
            "v*-*",
        ],
    )
    .and_then(|d| parse_describe(&d))
    .or_else(|| {
        let commits = git(dir, &["rev-list", "--count", "HEAD"])?.parse().ok()?;
        let sha = git(dir, &["rev-parse", "--short=7", "HEAD"])?;
        is_hex(&sha).then_some(GitState::Untagged { commits, sha })
    });
    (state, watch)
}

/// The build script's half: sets `SCOOT_BUILD_VERSION` for the crate being
/// built and tells cargo when to work it out again.
pub fn emit() {
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from);
    let pkg_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    // Every binary crate sits at `crates/<name>/` under the workspace root.
    let (state, watch) = match manifest_dir.as_deref() {
        Some(dir) => match dir.parent().and_then(Path::parent) {
            Some(root) => git_state(dir, root),
            None => (None, Vec::new()),
        },
        None => (None, Vec::new()),
    };
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../scoot-ipc/build_label.rs");
    for path in watch {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!(
        "cargo:rustc-env=SCOOT_BUILD_VERSION={}",
        label(&pkg_version, state.as_ref())
    );
}
