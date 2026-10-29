//! Where a look's static leaf (`apps/helix.toml`, ...) lives on disk,
//! found without following a symlink.
//!
//! `scoot-look build` and `scoot look apply` both read the leaf through
//! [`locate_leaf`], so one rule decides both: no component of the path
//! below the look directory may be a symlink, whether it points inside the
//! look, outside it, nowhere or in a loop. The look directory itself may be
//! one (a catalog entry linked to the store is how home-manager installs a
//! look); only what the look carries is checked.
//!
//! The check is a `symlink_metadata` walk, so a link swapped in between the
//! walk and the read is the reader's job: the leaf is opened `O_NOFOLLOW`
//! and an `ELOOP` refuses. A directory component swapped for a link in that
//! window is the one gap; it takes a write to the look directory, which is
//! already the same user's trust domain.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// The path of `rel` inside `base`, `Ok(None)` when it (or a directory on
/// the way to it) is absent, and an error for a symlink at any component
/// or a stat that fails for another reason (permissions, say). `rel` is a
/// normalized relative path (`check_relative_path` shape).
///
/// Presence only: the caller still reads the file with a no-follow open
/// that requires a regular file, so a directory or FIFO at the leaf refuses
/// there.
pub fn locate_leaf(base: &Path, rel: &str) -> Result<Option<PathBuf>, String> {
    let mut path = base.to_path_buf();
    let mut parts = rel.split('/').peekable();
    while let Some(part) = parts.next() {
        path.push(part);
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(error)
                if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(format!("cannot stat {}: {error}", path.display())),
        };
        if meta.is_symlink() {
            return Err(format!("refused: {} is a symlink", path.display()));
        }
        // A regular file where a directory belongs: nothing below it.
        if parts.peek().is_some() && !meta.is_dir() {
            return Ok(None);
        }
    }
    Ok(Some(path))
}

#[cfg(test)]
mod tests;
