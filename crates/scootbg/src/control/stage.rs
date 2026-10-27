//! Publishing the control socket: `0600` from the moment its name exists.
//!
//! `bind` creates the socket with `0777 & !umask`, so a `chmod` after it
//! leaves a window where the published name holds a socket other users can
//! connect to (`0755` under the common `022`). Instead the socket is built
//! out of sight and renamed onto its name, as scoot's IPC listener and
//! scootlaunch do:
//!
//! - a freshly-made `0700` staging directory beside the path, with an
//!   unpredictable name;
//! - reopened `O_DIRECTORY | O_NOFOLLOW` and pinned by fd, `fchmod`ed to
//!   `0700` through it (a odd umask can only tighten `mkdir`'s mode), and
//!   the socket bound under `/proc/self/fd/<n>`, so no component of the
//!   path can be swapped between the check and the bind;
//! - `chmod 0600` while still unreachable, then an atomic `rename` onto the
//!   published name. The name is therefore only ever the previous socket or
//!   this `0600` one, and a stale socket is replaced without the name going
//!   missing.
//!
//! (As scootui's `control/stage.rs`, which this duplicates: scootbg is a
//! binary crate that links nothing of scootui's.)

use std::io;
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

use rustix::fs::{Mode, OFlags};

/// `sun_path` holds 108 bytes including its NUL: 107 is the longest path a
/// unix socket can bind at.
const SUN_PATH_MAX: usize = 107;
/// The socket is `0600` before it is reachable.
const SOCKET_MODE: u32 = 0o600;
/// The staging directory is `0700`: owner only.
const STAGING_MODE: u32 = 0o700;
/// Staging names to try before giving up.
const STAGING_ATTEMPTS: usize = 16;

/// Binds a nonblocking listener and publishes it at `path` as `0600`,
/// replacing whatever socket is there in one `rename`.
pub(super) fn bind(path: &Path) -> io::Result<UnixListener> {
    // The bind no longer happens at `path`, so a path too long for
    // `sun_path` would bind and rename happily and leave a socket no client
    // can connect to. Refuse it up front.
    if path.as_os_str().len() > SUN_PATH_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "the socket path is {} bytes; a unix socket path can be at most {SUN_PATH_MAX}",
                path.as_os_str().len()
            ),
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "the socket path names no directory to create it in",
        )
    })?;
    let mut attempts = STAGING_ATTEMPTS;
    loop {
        let staging = parent.join(staging_name());
        let claimed = std::fs::DirBuilder::new()
            .mode(STAGING_MODE)
            .create(&staging);
        attempts -= 1;
        match claimed {
            Ok(()) => return publish(&staging, path),
            // Taken by anything at all: another name, never a removal.
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && attempts > 0 => {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Builds the socket inside `staging` and publishes it at `path`, then
/// removes `staging` whether that worked or not. `rmdir` neither follows a
/// symlink nor removes a non-empty directory, so this removes the directory
/// just made and very little else.
fn publish(staging: &Path, path: &Path) -> io::Result<UnixListener> {
    let result = stage(staging, path);
    let _ = std::fs::remove_dir(staging);
    result
}

/// `directory` stays alive through the rename: `staged` borrows its fd
/// number, and reusing that number after an early drop would silently
/// operate on the wrong file.
fn stage(staging: &Path, path: &Path) -> io::Result<UnixListener> {
    let directory = rustix::fs::open(
        staging,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    rustix::fs::fchmod(&directory, Mode::from_bits_truncate(STAGING_MODE))?;
    let staged = PathBuf::from(format!("/proc/self/fd/{}/s", directory.as_raw_fd()));
    let listener = UnixListener::bind(&staged)?;
    let published = (|| {
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(SOCKET_MODE))?;
        listener.set_nonblocking(true)?;
        std::fs::rename(&staged, path)
    })();
    if let Err(error) = published {
        // Inside the pinned directory: the socket bound just above.
        let _ = std::fs::remove_file(&staged);
        return Err(error);
    }
    Ok(listener)
}

/// An unpredictable staging name (a pid alone is guessable in the shared
/// runtime dir), from the kernel's random pool, with the pid and a counter
/// mixed in so a `getrandom` failure still yields a unique one.
fn staging_name() -> PathBuf {
    use rustix::rand::GetRandomFlags;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut bytes = [0u8; 12];
    let mut filled = 0;
    while filled < bytes.len() {
        match rustix::rand::getrandom(&mut bytes[filled..], GetRandomFlags::empty()) {
            Ok(read) if read > 0 => filled += read,
            _ => break,
        }
    }
    let mut name = format!(
        ".scootbg-{}-{}-",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    name.extend(
        bytes[..filled]
            .iter()
            .map(|byte| char::from(ALPHABET[usize::from(*byte) % ALPHABET.len()])),
    );
    if filled == 0 {
        name.push('x');
    }
    PathBuf::from(name)
}
