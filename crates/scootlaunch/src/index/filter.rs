//! `OnlyShowIn`/`NotShowIn` and `TryExec`: which entries show.
//!
//! Against `$XDG_CURRENT_DESKTOP` (colon-separated, considered in
//! order): the current desktops are walked in order, and the first one
//! named in either list decides — named in `NotShowIn` hides, named in
//! `OnlyShowIn` shows. Past every desktop with neither naming it, a
//! non-empty `OnlyShowIn` hides. No `XDG_CURRENT_DESKTOP` means no
//! filtering. `TryExec` is looked up on `$PATH` at index time: a
//! missing or non-executable target hides the entry.
//!
//! A list key that refuses to parse is ignored (a parse error for that
//! key: the key is ignored, the entry kept).

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

/// Why a hidden entry is hidden. Rendered into `--list --all --json`
/// as the `hidden` reason, so an agent asking "an app is missing" can
/// be pointed at the cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HiddenReason {
    NoDisplay,
    OnlyShowIn,
    NotShowIn,
    TryExec,
}

impl HiddenReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            HiddenReason::NoDisplay => "NoDisplay",
            HiddenReason::OnlyShowIn => "OnlyShowIn",
            HiddenReason::NotShowIn => "NotShowIn",
            HiddenReason::TryExec => "TryExec",
        }
    }

    /// Parses a reason back (the cache round-trips it). Anything else
    /// is a corrupt cache: a rescan, never an error.
    pub fn from_str(text: &str) -> Option<Self> {
        match text {
            "NoDisplay" => Some(HiddenReason::NoDisplay),
            "OnlyShowIn" => Some(HiddenReason::OnlyShowIn),
            "NotShowIn" => Some(HiddenReason::NotShowIn),
            "TryExec" => Some(HiddenReason::TryExec),
            _ => None,
        }
    }
}

/// Answers the `OnlyShowIn`/`NotShowIn` verdict for one entry's group:
/// hidden by which list, if either. `current` is the ordered
/// `$XDG_CURRENT_DESKTOP` value.
pub fn show_in_hidden(group: &super::parse::Group, current: &[String]) -> Option<HiddenReason> {
    let only = list_key(group, "OnlyShowIn").unwrap_or_default();
    let not = list_key(group, "NotShowIn").unwrap_or_default();
    if only.is_empty() && not.is_empty() {
        return None;
    }
    for desktop in current {
        if not.iter().any(|entry| entry == desktop) {
            return Some(HiddenReason::NotShowIn);
        }
        if only.iter().any(|entry| entry == desktop) {
            return None;
        }
    }
    if only.is_empty() {
        None
    } else {
        Some(HiddenReason::OnlyShowIn)
    }
}

/// A `;`-separated list key, unescaped. Unparseable (`None`) means the
/// key is ignored, never that the entry hides.
fn list_key(group: &super::parse::Group, key: &str) -> Option<Vec<String>> {
    let raw = group.get(key)?;
    super::parse::split_list(raw).ok()
}

/// Answers whether `TryExec` hides the entry: `None` shows (no
/// `TryExec`, or an empty one, which is meaningless). Otherwise the
/// first whitespace-separated token is looked up — directly when it
/// holds a `/`, on `path_dirs` when it does not — and a missing or
/// non-executable target hides with [`HiddenReason::TryExec`].
pub fn try_exec_hidden(group: &super::parse::Group, path_dirs: &[PathBuf]) -> Option<HiddenReason> {
    let raw = group.get("TryExec")?;
    let target = raw.split([' ', '\t']).next().unwrap_or("");
    if target.is_empty() {
        return None;
    }
    let candidate = Path::new(target);
    if target.contains('/') {
        return match is_executable(candidate) {
            true => None,
            false => Some(HiddenReason::TryExec),
        };
    }
    for dir in path_dirs {
        if is_executable(&dir.join(target)) {
            return None;
        }
    }
    Some(HiddenReason::TryExec)
}

/// An executable file the launcher could spawn: a regular file
/// (never a directory, which the kernel's execute bit also grants
/// for search) with the execute bit the kernel would grant.
/// `rustix::fs::access` answers for the real uid/gid, including ACLs
/// and capabilities, where a mode-bits read would lie.
fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.is_file())
        .unwrap_or(false)
        && rustix::fs::access(path, rustix::fs::Access::EXEC_OK).is_ok()
}
