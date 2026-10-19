//! Run mode: executables on `$PATH`, ranked like apps.
//!
//! `--run` (alias `--list-executables-in-path`, fuzzel's flag) opens
//! the panel over every executable file found by scanning `$PATH` in
//! order: the first directory naming a basename wins, the rest are
//! ignored. Each entry launches by its basename (a `PATH` lookup at
//! spawn, through [`spawn::launch_command`](crate::spawn::launch_command)),
//! and frecency keys it by `run:<basename>`, as the history file's
//! docs say — the same key the empty-query order and the typed-query
//! bonus read.
//!
//! Bounds: at most [`MAX_EXES`] names. Unreadable directories,
//! entries that vanish mid-scan, broken symlinks and non-executable
//! files are skipped silently (a stale `$PATH` entry is routine, not
//! an error). Names decode lossily; a name that cannot spawn (NUL, a
//! vanished file) draws its spawn error in the panel like any failed
//! launch.

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// At most this many executable names are kept: past it the rest is
/// dropped, in scan order.
pub const MAX_EXES: usize = 65_536;

/// The raw `$PATH` directories, in order (from
/// [`Env`](crate::index::Env), so tests inject their own).
pub fn executable_names(path_dirs: &[PathBuf]) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for dir in path_dirs {
        if names.len() >= MAX_EXES {
            break;
        }
        collect_dir(dir, &mut names, &mut seen);
    }
    names
}

/// One directory's executables, in directory order: regular files
/// with any execute bit set (symlinks followed via the path, so a
/// link to an executable counts; a broken link or a loop is skipped
/// with the unreadable rest). Directories never launch.
fn collect_dir(dir: &Path, names: &mut Vec<String>, seen: &mut HashSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if names.len() >= MAX_EXES {
            return;
        }
        // `DirEntry::metadata` does not follow symlinks (a link reads
        // as non-file), so stat the path: this follows to the target.
        let Ok(meta) = std::fs::metadata(entry.path()) else {
            continue;
        };
        if meta.is_dir() || !is_executable(&meta) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if seen.insert(name.clone()) {
            names.push(name);
        }
    }
}

/// A regular file with any execute bit: the owner's, the group's, or
/// the other's. `std`-only (`PermissionsExt`), no `libc` crate.
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    meta.is_file() && meta.permissions().mode() & 0o111 != 0
}

/// Builds the panel's entries for `names`: one row per basename, the
/// id carrying the `run:` frecency key, the argv its basename (a
/// `PATH` lookup at spawn). Allocates (the open, not the keystroke).
pub fn to_entries(names: &[String]) -> Vec<crate::index::Entry> {
    names
        .iter()
        .map(|name| crate::index::Entry {
            id: format!("run:{name}"),
            name: name.clone(),
            generic_name: None,
            comment: None,
            keywords: Vec::new(),
            icon: None,
            exec: vec![name.clone()],
            terminal: false,
            dbus_activatable: false,
            actions: Vec::new(),
            path: PathBuf::new(),
            workdir: None,
            hidden: None,
        })
        .collect()
}
