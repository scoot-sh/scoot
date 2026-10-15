//! The look catalog: where source looks live, in search order, on any box.
//!
//! The order is the design's section 3.1 (user-looks, installed-looks,
//! home-manager, system, shipped), and it is the same with and without Nix:
//! a plain binary install carries its catalog as directories of plain
//! files, and Nix is only one way to have rendered and installed them.
//! Origin is by directory, never by manifest claim: only the lookup of the
//! shipped directories differs between a Nix and a non-Nix box, and that
//! lookup is here, not in the pure crate.
//!
//! A name that exists twice resolves to the earliest in the search order;
//! the later one is reported as shadowed, never merged.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use scoot_look::state::read_capped_follow;
use scoot_look::{
    Origin, RawLook, check_name_matches_dir, parse_look_toml, validate_apps_allowlist,
    validate_for_origin,
};

/// One source look found in the catalog.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The look's name (its `name`, matching its directory).
    pub name: String,
    /// The canonicalized source directory.
    pub dir: PathBuf,
    /// The canonical directory of the `look.toml` file itself: usually
    /// `dir`, but a home-manager `recursive` farm links each file into the
    /// store, so the manifest (and the files it names) live elsewhere.
    /// Look-relative files (wallpaper, `[fonts.files]`) resolve against
    /// this directory — the directory of the look that names them — so
    /// per-file farms catalog with the same inside-check as plain ones.
    pub manifest_dir: PathBuf,
    /// Decided by the directory, never by the manifest.
    pub origin: Origin,
    /// The parsed manifest.
    pub raw: RawLook,
    /// The exact `look.toml` bytes (the render cache is keyed on these).
    pub bytes: Vec<u8>,
}

/// The catalog: winning entries in search order, the shadowed names, and
/// notes (unreadable directories, invalid manifests) for the human reply.
#[derive(Debug, Default)]
pub struct Catalog {
    /// One entry per winning name, in search order.
    pub entries: Vec<Entry>,
    /// Names that existed more than once: the later directories, in order.
    pub shadowed: Vec<(String, PathBuf)>,
    /// Directories skipped and manifests refused, with reasons.
    pub notes: Vec<String>,
}

impl Catalog {
    /// The winning entry for `name`, if any.
    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    /// The `(RawLook, Origin)` map `resolve` takes, over winning entries.
    pub fn sources(&self) -> BTreeMap<String, (RawLook, Origin)> {
        self.entries
            .iter()
            .map(|entry| (entry.name.clone(), (entry.raw.clone(), entry.origin)))
            .collect()
    }
}

/// The data home: `$XDG_DATA_HOME`, else `~/.local/share`. Empty counts as
/// unset. `None` only when no home resolves at all.
pub fn data_home(xdg: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    let non_empty = |v: &&OsStr| !v.is_empty();
    if let Some(dir) = xdg.filter(non_empty) {
        return Some(PathBuf::from(dir));
    }
    home.filter(non_empty)
        .map(|dir| PathBuf::from(dir).join(".local/share"))
}

/// The cache home: `$XDG_CACHE_HOME`, else `~/.cache`. Empty counts as
/// unset. `None` only when no home resolves at all.
pub fn cache_home(xdg: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    let non_empty = |v: &&OsStr| !v.is_empty();
    if let Some(dir) = xdg.filter(non_empty) {
        return Some(PathBuf::from(dir));
    }
    home.filter(non_empty)
        .map(|dir| PathBuf::from(dir).join(".cache"))
}

/// The config home: `$XDG_CONFIG_HOME`, else `~/.config`. Empty counts as
/// unset. `None` only when no home resolves at all.
pub fn config_home(xdg: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    let non_empty = |v: &&OsStr| !v.is_empty();
    if let Some(dir) = xdg.filter(non_empty) {
        return Some(PathBuf::from(dir));
    }
    home.filter(non_empty)
        .map(|dir| PathBuf::from(dir).join(".config"))
}

/// The shipped source directories, in order: an explicit `$SCOOT_LOOKS_DIR`
/// first (tests and developers), then `<prefix>/share/scoot/looks` beside
/// this binary (a tagged-release tarball, `install.sh` or a distro package
/// lays the catalog there), then the well-known prefixes. Missing
/// directories are skipped, never an error.
pub fn shipped_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("SCOOT_LOOKS_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
    {
        dirs.push(dir);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(bin) = exe.parent()
        && let Some(prefix) = bin.parent()
    {
        dirs.push(prefix.join("share/scoot/looks"));
    }
    dirs.push(PathBuf::from("/usr/local/share/scoot/looks"));
    dirs.push(PathBuf::from("/usr/share/scoot/looks"));
    dirs
}

/// Discovers the catalog over the live environment.
pub fn discover() -> Catalog {
    discover_with(&shipped_dirs())
}

/// Discovers the catalog with an explicit shipped set, so tests pin the
/// shipped half without touching the process environment or the binary's
/// own prefix.
pub fn discover_with(shipped: &[PathBuf]) -> Catalog {
    let xdg_data = std::env::var_os("XDG_DATA_HOME");
    let home = std::env::var_os("HOME");
    let data = data_home(xdg_data.as_deref(), home.as_deref());
    let home_share = home
        .as_deref()
        .filter(|v| !v.is_empty())
        .map(|dir| PathBuf::from(dir).join(".local/share"));

    // (directory, origin), in search order.
    let mut ordered: Vec<(PathBuf, Origin)> = Vec::new();
    if let Some(data) = &data {
        ordered.push((data.join("scoot/user-looks"), Origin::User));
        ordered.push((data.join("scoot/installed-looks"), Origin::Installed));
    }
    if let Some(share) = &home_share {
        ordered.push((share.join("scoot/looks"), Origin::Declared));
    }
    ordered.push((
        PathBuf::from("/run/current-system/sw/share/scoot/looks"),
        Origin::Declared,
    ));
    for dir in shipped {
        ordered.push((dir.clone(), Origin::Shipped));
    }
    discover_ordered(&ordered)
}

/// Discovers the catalog over an explicit ordered directory list, so tests
/// pin the search without touching the process environment. The same
/// directory twice scans once: a look must not shadow itself.
pub fn discover_ordered(ordered: &[(PathBuf, Origin)]) -> Catalog {
    let mut seen_dirs: Vec<PathBuf> = Vec::new();
    let mut catalog = Catalog::default();
    for (dir, origin) in ordered {
        let canonical = match std::fs::canonicalize(dir) {
            Ok(canonical) => canonical,
            Err(_) => continue,
        };
        if seen_dirs.contains(&canonical) {
            continue;
        }
        seen_dirs.push(canonical.clone());
        scan_dir(&canonical, *origin, &mut catalog);
    }
    catalog
}

/// Scans one source directory: each child directory with a readable
/// `look.toml` that parses, validates and matches its directory name
/// becomes a candidate. Anything else is a note, never a failure: one
/// broken look must not hide the catalog.
fn scan_dir(dir: &Path, origin: Origin, catalog: &mut Catalog) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    // Sorted, so the listing is deterministic whatever the filesystem
    // returns.
    let mut names: Vec<_> = entries.collect();
    names.sort_by_key(|entry| entry.as_ref().map(|e| e.file_name()).unwrap_or_default());
    for entry in names {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        // A home-manager catalog is symlinks into the store: a symlink to
        // a directory is a look. Anything else (files, FIFOs) is skipped
        // quietly here and refused loudly by `install`, which owns that
        // trust boundary.
        if !(meta.is_dir() || meta.is_symlink()) {
            continue;
        }
        let canonical = match std::fs::canonicalize(&path) {
            Ok(canonical) => canonical,
            Err(error) => {
                catalog.notes.push(format!(
                    "skipping {}: cannot resolve it: {error}",
                    path.display()
                ));
                continue;
            }
        };
        if !canonical.is_dir() {
            continue;
        }
        // Home-manager/Nix-style catalogs symlink each look: the name is
        // the entry's own name, never the store target's basename (which
        // carries a hash prefix like `<hash>-moonrise-look`).
        let dir_name = entry.file_name().to_str().unwrap_or_default().to_owned();
        // One canonicalize for both the read and the manifest dir: the
        // manifest itself may be a store link (a per-file farm), so resolve
        // the file once, read through the resolved path, and take its
        // parent as the manifest dir. Falls back to the entry dir when the
        // target has no parent (unreachable: it names `look.toml`) or does
        // not resolve (the read below notes it).
        let file = canonical.join("look.toml");
        // Home-manager `recursive` farms link each file into the store, so
        // the manifest itself may be a symlink: follow it here (the trust
        // boundary is the catalog directory, and the manifest is parsed and
        // validated, never executed). Untrusted directory inputs under
        // `validate`/`build` keep the no-follow read.
        let manifest_file = std::fs::canonicalize(&file).unwrap_or_else(|_| file.clone());
        let bytes = match read_capped_follow(&manifest_file) {
            Ok(text) => text.into_bytes(),
            Err(error) => {
                catalog.notes.push(format!(
                    "skipping {dir_name}: cannot read {}: {error}",
                    manifest_file.display()
                ));
                continue;
            }
        };
        if bytes.len() as u64 > scoot_look::state::MAX_FILE {
            catalog.notes.push(format!(
                "skipping {dir_name}: look.toml is larger than {} bytes",
                scoot_look::state::MAX_FILE
            ));
            continue;
        }
        let raw = match parse_look_toml(&bytes) {
            Ok(raw) => raw,
            Err(errors) => {
                let first = errors
                    .first()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "cannot parse".to_owned());
                catalog
                    .notes
                    .push(format!("skipping {dir_name}: invalid look.toml: {first}"));
                continue;
            }
        };
        if let Some(error) = check_name_matches_dir(&raw, &dir_name) {
            catalog.notes.push(format!("skipping {dir_name}: {error}"));
            continue;
        }
        let mut failed = false;
        for error in validate_for_origin(&raw, origin)
            .into_iter()
            .chain(validate_apps_allowlist(&raw.apps))
        {
            catalog.notes.push(format!(
                "skipping {}: invalid for origin {}: {error}",
                raw.name,
                origin_name(origin)
            ));
            failed = true;
        }
        if failed {
            continue;
        }
        // Contrast is resolved over the catalog at the use site (an
        // `extends` child needs its base); the winner check below runs on
        // names only.
        if catalog.find(&raw.name).is_some() {
            catalog.shadowed.push((raw.name.clone(), canonical));
            continue;
        }
        // The manifest's own directory: the file read above may itself be
        // a store link, and the files it names sit beside the target, not
        // the entry (`manifest_file` above already resolved it once, so no
        // second canonicalize here).
        let manifest_dir = manifest_file
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| canonical.clone());
        catalog.entries.push(Entry {
            name: raw.name.clone(),
            dir: canonical,
            manifest_dir,
            origin,
            raw,
            bytes,
        });
    }
}

/// The origin's name, as the listing shows it.
pub fn origin_name(origin: Origin) -> &'static str {
    match origin {
        Origin::Shipped => "shipped",
        Origin::Declared => "declared",
        Origin::User => "user",
        Origin::Installed => "installed",
    }
}

#[cfg(test)]
pub(crate) mod tests;
