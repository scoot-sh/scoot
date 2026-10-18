//! Data dirs and ids, with precedence and `Hidden` masking.
//!
//! `$XDG_DATA_HOME/applications` (default `~/.local/share`) first, then
//! each `$XDG_DATA_DIRS` entry's `applications` (default
//! `/usr/local/share/:/usr/share/`). Relative entries are ignored.
//! Subdirectories are walked, because they form ids: the path relative
//! to the data dir, minus `applications/`, with `/` turned into `-`.
//! Only `.desktop` files form entries (`mimeinfo.cache` and friends
//! are skipped silently).
//!
//! Symlinks are followed, files and directories alike (Nix profiles
//! are symlink trees, and `buildEnv` links whole directories). Loops
//! are cut by a visited `(dev, ino)` set, and depth is capped at 8.
//!
//! This file is `std`-plus-`rustix` only (file types and identity).

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

/// A subdirectory loop, or the one file that broke a bound: the index
/// prints these as debug lines, never errors.
pub type Note = String;

/// One `applications` directory, resolved and stated.
#[derive(Debug, Clone)]
pub struct AppDir {
    /// `<data dir>/applications`, as walked.
    pub dir: PathBuf,
    /// The same directory canonicalized (NixOS: through the profile
    /// symlink into the store). The cache key compares these.
    pub canonical: PathBuf,
    pub dev: u64,
    pub ino: u64,
    pub mtime: std::time::SystemTime,
}

/// One desktop file found by the walk: its id and its path,
/// through whatever symlinks led to it. File identity for the cache
/// key is recorded separately at walk time ([`KeyEntry`]), so this
/// carries only what the entry builder needs.
#[derive(Debug, Clone)]
pub struct FoundFile {
    /// The desktop file id (`foo/bar.desktop` → `foo-bar.desktop`).
    pub id: String,
    /// The file, through whatever symlinks led to it.
    pub path: PathBuf,
}

/// How deep below an `applications` directory the walk descends.
pub const MAX_DEPTH: u32 = 8;

/// Resolves the data dirs in precedence order: `XDG_DATA_HOME` (or
/// `~/.local/share`) ahead of every `XDG_DATA_DIRS` entry (or the two
/// defaults). Empty and relative entries are ignored, as is a missing
/// `applications` subdirectory (no dir, no entries, no error).
pub fn resolve_app_dirs(env: &super::Env) -> Vec<AppDir> {
    let mut dirs = Vec::new();
    if let Some(home) = &env.xdg_data_home {
        push_app_dir(home, &mut dirs);
    }
    for dir in &env.xdg_data_dirs {
        push_app_dir(dir, &mut dirs);
    }
    // The same directory twice (a profile linking the system dir, a
    // repeated entry) would index every id twice and note every id as
    // a collision. Exact duplicates go; the precedence logic below
    // still guards two spellings of one directory.
    let mut seen = HashSet::new();
    dirs.retain(|dir: &AppDir| seen.insert(dir.dir.clone()));
    dirs
}

/// The default `XDG_DATA_DIRS`, when unset or empty.
pub const DEFAULT_DATA_DIRS: &[&str] = &["/usr/local/share/", "/usr/share/"];

fn push_app_dir(data_dir: &Path, out: &mut Vec<AppDir>) {
    let dir = data_dir.join("applications");
    // The walk needs the directory's identity for the visited set
    // and the cache key; a missing dir is not an error.
    let metadata = match std::fs::metadata(&dir) {
        Ok(metadata) => metadata,
        Err(_) => return,
    };
    if !metadata.is_dir() {
        return;
    }
    // Canonicalize for the NixOS case: `~/.nix-profile/share` is a
    // symlink into the store, and every store directory has mtime 1,
    // so a profile switch changes this path and nothing else. A
    // directory that will not canonicalize keeps its walked path.
    let canonical = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
    out.push(AppDir {
        dir,
        canonical,
        dev: metadata.dev(),
        ino: metadata.ino(),
        mtime: metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
    });
}

/// Walks one `applications` directory depth-first, entries sorted by
/// name so the output is deterministic. Answers the desktop files
/// with their ids, the identity entries for the cache key (every
/// directory descended plus every regular file seen, `.desktop` or
/// not), and debug notes.
pub fn walk(app: &AppDir) -> (Vec<FoundFile>, Vec<KeyEntry>, Vec<Note>) {
    let mut files = Vec::new();
    let mut key = Vec::new();
    let mut notes = Vec::new();
    let mut visited = HashSet::new();
    visited.insert((app.dev, app.ino));
    key.push(KeyEntry::dir(&app.canonical, app.dev, app.ino, app.mtime));
    let mut stack = vec![(app.dir.clone(), 0u32)];
    while let Some((dir, depth)) = stack.pop() {
        let listing = match std::fs::read_dir(&dir) {
            Ok(listing) => listing,
            Err(error) => {
                notes.push(format!("{}: cannot list directory: {error}", dir.display()));
                continue;
            }
        };
        let mut entries = Vec::new();
        for entry in listing {
            match entry {
                Ok(entry) => entries.push(entry),
                Err(error) => notes.push(format!("{}: cannot read entry: {error}", dir.display())),
            }
        }
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            // `symlink_metadata` first: a symlink is followed only
            // below, on purpose. A dead link, and anything stranger
            // than a file or a directory, is skipped with a note.
            let link_meta = match std::fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(error) => {
                    notes.push(format!("{}: cannot stat: {error}", path.display()));
                    continue;
                }
            };
            let (meta, followed) = if link_meta.is_symlink() {
                match std::fs::metadata(&path) {
                    Ok(meta) => (meta, true),
                    Err(_) => {
                        notes.push(format!("{}: dangling symlink, skipped", path.display()));
                        continue;
                    }
                }
            } else {
                (link_meta, false)
            };
            if meta.is_dir() {
                if depth >= MAX_DEPTH {
                    notes.push(format!(
                        "{}: deeper than {MAX_DEPTH}, skipped",
                        path.display()
                    ));
                    continue;
                }
                let identity = (meta.dev(), meta.ino());
                if !visited.insert(identity) {
                    notes.push(format!(
                        "{}: symlink loop (already visited), skipped",
                        path.display()
                    ));
                    continue;
                }
                let canonical = if followed {
                    std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone())
                } else {
                    path.clone()
                };
                key.push(KeyEntry::dir(
                    &canonical,
                    meta.dev(),
                    meta.ino(),
                    meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                ));
                stack.push((path, depth + 1));
            } else if meta.is_file() {
                key.push(KeyEntry::file(
                    &path,
                    meta.dev(),
                    meta.ino(),
                    meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                ));
                let Some(id) = file_id(&app.dir, &path) else {
                    // Not a `.desktop` file (`mimeinfo.cache`, icon
                    // themes dropped in the wrong place): skipped
                    // silently, as the spec's tooling does.
                    continue;
                };
                files.push(FoundFile { id, path });
            } else {
                notes.push(format!(
                    "{}: not a file or directory, skipped",
                    path.display()
                ));
            }
        }
    }
    (files, key, notes)
}

/// The id of one file: its path relative to the `applications`
/// directory, with `/` turned into `-`. Only `.desktop` files have
/// ids; anything else answers `None`.
fn file_id(app_dir: &Path, path: &Path) -> Option<String> {
    if path.extension().and_then(|ext| ext.to_str()) != Some("desktop") {
        return None;
    }
    let relative = path.strip_prefix(app_dir).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        let bytes = component.as_os_str().as_bytes();
        // A non-UTF-8 name cannot form an id: skipped, never decoded
        // lossily into a wrong one.
        parts.push(std::str::from_utf8(bytes).ok()?);
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("-"))
}

/// One identity the cache key compares: every directory descended
/// (canonical path) plus every regular file seen, each with the
/// `(dev, ino, mtime)` that notices replacement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyEntry {
    Dir {
        path: PathBuf,
        dev: u64,
        ino: u64,
        mtime: std::time::SystemTime,
    },
    File {
        path: PathBuf,
        dev: u64,
        ino: u64,
        mtime: std::time::SystemTime,
    },
}

impl KeyEntry {
    fn dir(path: &Path, dev: u64, ino: u64, mtime: std::time::SystemTime) -> Self {
        KeyEntry::Dir {
            path: path.to_path_buf(),
            dev,
            ino,
            mtime,
        }
    }

    fn file(path: &Path, dev: u64, ino: u64, mtime: std::time::SystemTime) -> Self {
        KeyEntry::File {
            path: path.to_path_buf(),
            dev,
            ino,
            mtime,
        }
    }
}
