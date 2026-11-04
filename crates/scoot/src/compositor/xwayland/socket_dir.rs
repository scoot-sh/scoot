//! The directory X servers keep their sockets in, `/tmp/.X11-unix`.
//!
//! Smithay binds the server's listening sockets itself, before it execs
//! `Xwayland`, and its bind does not create the directory. A real X server
//! (or `systemd-tmpfiles`) usually has by then; a machine where none ever
//! has -- a fresh container, a minimal CI runner -- fails every display
//! number in turn, and Smithay reports the whole scan as "Could not find a
//! free socket for the XServer" (`AddrInUse`), which names neither the
//! directory nor the real error. The server itself would have made the
//! directory (Xtrans does, mode 1777), so scoot does the same first.

use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::Path;

/// Where Smithay puts its sockets (`x11_sockets.rs` hard-codes it).
pub(super) const X11_SOCKET_DIR: &str = "/tmp/.X11-unix";

/// Makes sure `dir` exists, creating it world-writable with the sticky bit
/// (1777) the way an X server does, whatever the umask. An existing
/// directory is left as it is: not chmod-ed, not checked for an owner.
///
/// Racing creators are fine: losing the race to another scoot or to a real
/// X server is success. A non-directory in the way is the caller's `Err`.
pub(super) fn ensure(dir: &Path) -> io::Result<()> {
    match std::fs::DirBuilder::new().mode(0o1777).create(dir) {
        Ok(()) => {
            // `mkdir` applies the umask; the sticky world-writable mode is
            // what lets every user's server share the directory.
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o1777))
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if dir.is_dir() {
                Ok(())
            } else {
                Err(error)
            }
        }
        Err(error) => Err(error),
    }
}

/// [`ensure`] for [`X11_SOCKET_DIR`], logging a failure rather than
/// returning it: the spawn that follows fails on its own if the directory
/// really is unusable, and its error is the one the caller reports.
pub(super) fn ensure_default() {
    if let Err(error) = ensure(Path::new(X11_SOCKET_DIR)) {
        tracing::warn!(dir = X11_SOCKET_DIR, %error, "could not create the X socket directory");
    }
}

#[cfg(test)]
mod tests;
