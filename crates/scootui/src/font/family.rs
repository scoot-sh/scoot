//! Finding a look's font family as files, without fontconfig.
//!
//! A look names a family (`DejaVu Sans`, `DroidSansM Nerd Font Propo`); the
//! shell apps draw from files. This walks the usual font directories once,
//! at load or reload, for a file whose name is the family's: the regular
//! face and, from the same font directory root, the bold. It is a match on file names, not a
//! reading of font tables, so it opens nothing it does not use.
//!
//! A name matches when, with case, spaces, hyphens and underscores dropped,
//! the file stem is the family (`DejaVuSans.ttf`), the family and
//! `regular` (`NotoSans-Regular.ttf`) for the regular face, or the family
//! and `bold` (`DejaVuSans-Bold.ttf`) for the bold one. Only `.ttf` and
//! `.otf` files are taken; collections (`.ttc`) and styles in between
//! (semibold, medium) are not.
//!
//! The walk is bounded ([`MAX_DEPTH`], [`MAX_ENTRIES`]), so a symlink loop
//! or an enormous tree costs a bounded start-up, never a hang. Symlinks are
//! followed, since a Nix profile's font directory is made of them.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

/// The deepest directory descended into, below each font directory.
pub const MAX_DEPTH: usize = 6;
/// The most directory entries looked at in one search, across every
/// directory.
pub const MAX_ENTRIES: usize = 100_000;

/// The files of one family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Family {
    pub regular: PathBuf,
    /// The bold file found with the regular one: in the same font directory
    /// root, not necessarily the same folder; if one was asked for and
    /// exists.
    pub bold: Option<PathBuf>,
}

/// `name` for matching: lowercase letters and digits only.
fn squash(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The directories a family is looked for in, most specific first: the
/// user's, then the system's, then what the distributions and Nix profiles
/// link in. Only those that exist are returned, each once.
pub fn font_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from);
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
    let data_dirs = std::env::var_os("XDG_DATA_DIRS")
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let mut dirs: Vec<PathBuf> = Vec::new();
    dirs.extend(data_home.map(|d| d.join("fonts")));
    dirs.extend(home.iter().map(|h| h.join(".fonts")));
    dirs.extend(std::env::split_paths(&data_dirs).map(|d| d.join("fonts")));
    dirs.extend(home.iter().map(|h| h.join(".nix-profile/share/fonts")));
    if let Some(user) = std::env::var_os("USER").filter(|u| !u.is_empty()) {
        dirs.push(
            Path::new("/etc/profiles/per-user")
                .join(user)
                .join("share/fonts"),
        );
    }
    dirs.extend(
        [
            "/usr/share/fonts",
            "/usr/local/share/fonts",
            "/run/current-system/sw/share/X11/fonts",
            "/run/current-system/sw/share/fonts",
        ]
        .map(PathBuf::from),
    );
    let mut seen: Vec<PathBuf> = Vec::new();
    dirs.retain(|dir| {
        let fresh = dir.is_absolute() && !seen.contains(dir) && dir.is_dir();
        if fresh {
            seen.push(dir.clone());
        }
        fresh
    });
    dirs
}

/// Which face a file name is, for the squashed `family`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Regular,
    Bold,
}

/// The face `file` is of `family` (squashed), by its name alone.
fn style_of(file: &OsStr, family: &str) -> Option<Style> {
    let path = Path::new(file);
    let ext = path.extension()?.to_str()?;
    if !(ext.eq_ignore_ascii_case("ttf") || ext.eq_ignore_ascii_case("otf")) {
        return None;
    }
    let stem = squash(path.file_stem()?.to_str()?);
    let rest = stem.strip_prefix(family)?;
    match rest {
        "" | "regular" => Some(Style::Regular),
        "bold" => Some(Style::Bold),
        _ => None,
    }
}

/// What a search is looking at, and how much of its budget is left.
struct Search {
    family: String,
    want_bold: bool,
    budget: usize,
}

/// The files of `family` in `dirs` (in priority order): the regular face
/// from the first font directory (root) that has one, the bold from that
/// same root when `want_bold`, in whatever folder below it. `None` when no directory has a regular
/// face. An unreadable directory, entry or symlink is skipped, never an
/// error.
pub fn find_family(family: &str, dirs: &[PathBuf], want_bold: bool) -> Option<Family> {
    let squashed = squash(family);
    if squashed.is_empty() {
        return None;
    }
    let mut search = Search {
        family: squashed,
        want_bold,
        budget: MAX_ENTRIES,
    };
    dirs.iter().find_map(|dir| {
        let mut regular = None;
        let mut bold = None;
        walk(dir, 0, &mut search, &mut regular, &mut bold);
        regular.map(|regular| Family { regular, bold })
    })
}

/// Whether the search under one directory has what it needs.
fn done(search: &Search, regular: &Option<PathBuf>, bold: &Option<PathBuf>) -> bool {
    regular.is_some() && (bold.is_some() || !search.want_bold)
}

/// Walks `dir`, remembering the first regular and bold file of the family.
/// Within a directory, the lexically first of several candidates wins, so
/// the answer does not depend on the filesystem's listing order.
fn walk(
    dir: &Path,
    depth: usize,
    search: &mut Search,
    regular: &mut Option<PathBuf>,
    bold: &mut Option<PathBuf>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut subdirs: Vec<PathBuf> = Vec::new();
    let (mut here_regular, mut here_bold): (Option<PathBuf>, Option<PathBuf>) = (None, None);
    for entry in entries {
        if search.budget == 0 {
            break;
        }
        search.budget -= 1;
        let Ok(entry) = entry else { continue };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        // The listing says what an entry is without a `stat`; only a
        // symlink (a Nix profile's fonts are links into the store) is
        // followed to learn what it points at.
        let (is_dir, is_file) = if kind.is_symlink() {
            match fs::metadata(entry.path()) {
                Ok(meta) => (meta.is_dir(), meta.is_file()),
                Err(_) => continue,
            }
        } else {
            (kind.is_dir(), kind.is_file())
        };
        if is_dir {
            if depth < MAX_DEPTH {
                subdirs.push(entry.path());
            }
        } else if is_file {
            let slot = match style_of(&entry.file_name(), &search.family) {
                Some(Style::Regular) => &mut here_regular,
                Some(Style::Bold) => &mut here_bold,
                None => continue,
            };
            let path = entry.path();
            if slot.as_ref().is_none_or(|old| path < *old) {
                *slot = Some(path);
            }
        }
    }
    if regular.is_none() {
        *regular = here_regular;
    }
    if search.want_bold && bold.is_none() {
        *bold = here_bold;
    }
    if done(search, regular, bold) {
        return;
    }
    // Subdirectories in name order, for the same stability.
    subdirs.sort_unstable();
    for sub in subdirs {
        if search.budget == 0 || done(search, regular, bold) {
            return;
        }
        walk(&sub, depth + 1, search, regular, bold);
    }
}

/// The bold file beside `regular`, by name: `DejaVuSans.ttf` and
/// `NotoSans-Regular.ttf` give `DejaVuSans-Bold.ttf` and
/// `NotoSans-Bold.ttf`. `None` if the name is not a regular face's or the
/// file is not there.
pub fn bold_sibling(regular: &Path) -> Option<PathBuf> {
    let stem = regular.file_stem()?.to_str()?;
    let ext = regular.extension()?.to_str()?;
    let base = ["-Regular", "_Regular", "Regular"]
        .iter()
        .find_map(|suffix| stem.strip_suffix(suffix))
        .unwrap_or(stem);
    if base.is_empty() || stem.ends_with("Bold") {
        return None;
    }
    let base = base.trim_end_matches(['-', '_']);
    let bold = regular.with_file_name(format!("{base}-Bold.{ext}"));
    bold.is_file().then_some(bold)
}

#[cfg(test)]
mod tests;
