//! Claiming the control socket: one daemon per display.
//!
//! 1. Take an exclusive, non-blocking `flock` on the lock file. If another
//!    process holds it, a daemon is alive: refuse.
//! 2. Holding the lock, any socket file at the path belongs to a daemon
//!    that died without removing it. Check that nothing answers on it (a
//!    daemon that somehow runs without the lock, say because someone
//!    deleted the lock file under it, still gets a refusal rather than its
//!    socket stolen), then remove it.
//! 3. Bind, listen, and make the socket owner-only.
//!
//! [`Claim::release`] (or, failing that, `Drop`) removes the socket file
//! and then drops the lock, so once a daemon has closed its clients'
//! connections a new one can start at once: `scootbg kill && scootbg
//! daemon` never races. A crash leaves the socket file, and the kernel
//! drops the lock.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

use rustix::fs::{FlockOperation, flock};
use rustix::io::Errno;
use rustix::net::{AddressFamily, SocketAddrUnix, SocketFlags, SocketType, connect, socket_with};

use crate::paths::Paths;

#[derive(Debug)]
pub enum ClaimError {
    /// A daemon already holds the lock for this display.
    AlreadyRunning { lock: PathBuf },
    /// Something answers on the socket although the lock was free.
    Answering { socket: PathBuf },
    /// Something other than a socket sits at the socket path.
    NotASocket { socket: PathBuf },
    Io {
        what: &'static str,
        path: PathBuf,
        error: io::Error,
    },
}

impl fmt::Display for ClaimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning { lock } => write!(
                f,
                "a scootbg daemon is already running for this display (it holds {}); \
                 `scootbg kill` stops it",
                lock.display()
            ),
            Self::Answering { socket } => write!(
                f,
                "something is already answering on {} although no daemon holds its \
                 lock; refusing to replace it",
                socket.display()
            ),
            Self::NotASocket { socket } => write!(
                f,
                "{} exists and is not a socket; refusing to remove it",
                socket.display()
            ),
            Self::Io { what, path, error } => {
                write!(f, "cannot {what} {}: {error}", path.display())
            }
        }
    }
}

impl std::error::Error for ClaimError {}

/// The claimed socket: the lock held, the listener bound.
#[derive(Debug)]
pub struct Claim {
    listener: UnixListener,
    socket: PathBuf,
    /// Held until `release`; `None` after it.
    lock: Option<File>,
}

impl Claim {
    pub fn acquire(paths: &Paths) -> Result<Self, ClaimError> {
        let lock = take_lock(&paths.lock)?;
        clear_stale(&paths.socket)?;
        let io_error = |what, error| ClaimError::Io {
            what,
            path: paths.socket.clone(),
            error,
        };
        let listener = UnixListener::bind(&paths.socket).map_err(|e| io_error("bind", e))?;
        // Built now so a failure below still removes the file.
        let claim = Self {
            listener,
            socket: paths.socket.clone(),
            lock: Some(lock),
        };
        // The runtime dir is already owner-only; this keeps the socket so
        // if the file is ever moved or the dir's mode is loose.
        fs::set_permissions(&paths.socket, fs::Permissions::from_mode(0o600))
            .map_err(|e| io_error("set permissions on", e))?;
        claim
            .listener
            .set_nonblocking(true)
            .map_err(|e| io_error("configure", e))?;
        Ok(claim)
    }

    pub fn listener(&self) -> &UnixListener {
        &self.listener
    }

    /// Removes the socket file, so no new client can connect, then drops
    /// the lock, so a new daemon may start. Idempotent. The socket goes
    /// first: while the lock is held, no other daemon can have bound the
    /// path, so the file removed is always this daemon's own.
    pub fn release(&mut self) {
        if let Some(lock) = self.lock.take() {
            let _ = fs::remove_file(&self.socket);
            drop(lock);
        }
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.release();
    }
}

/// Whether something is listening on `socket`, without blocking: a
/// blocking `connect` would wait for room in the backlog of a listener
/// that has stopped (`SIGSTOP`), hanging start-up. `EAGAIN` (backlog full)
/// counts as listening; `ECONNREFUSED` and `ENOENT` as not.
fn answers(socket: &Path) -> Result<bool, ClaimError> {
    let io_error = |errno: Errno| ClaimError::Io {
        what: "probe",
        path: socket.to_path_buf(),
        error: errno.into(),
    };
    let probe = socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    )
    .map_err(io_error)?;
    let address = SocketAddrUnix::new(socket).map_err(io_error)?;
    match connect(&probe, &address) {
        Ok(()) | Err(Errno::AGAIN) | Err(Errno::INPROGRESS) => Ok(true),
        Err(Errno::CONNREFUSED) | Err(Errno::NOENT) => Ok(false),
        Err(errno) => Err(io_error(errno)),
    }
}

/// This display's lock, if no daemon holds it (`None` if one does): held,
/// no daemon can start for the display until it is dropped. `apply-config`
/// holds it while it writes a profile's state with no daemon running, so
/// no daemon starting meanwhile reads the file before the write.
pub fn lock_if_free(path: &Path) -> Result<Option<File>, ClaimError> {
    match take_lock(path) {
        Ok(file) => Ok(Some(file)),
        Err(ClaimError::AlreadyRunning { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

fn take_lock(path: &Path) -> Result<File, ClaimError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(|error| ClaimError::Io {
            what: "open",
            path: path.to_path_buf(),
            error,
        })?;
    match flock(&file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(file),
        Err(Errno::WOULDBLOCK) => Err(ClaimError::AlreadyRunning {
            lock: path.to_path_buf(),
        }),
        Err(errno) => Err(ClaimError::Io {
            what: "lock",
            path: path.to_path_buf(),
            error: errno.into(),
        }),
    }
}

/// With the lock held: removes a dead daemon's socket, refusing if the
/// path is live or not a socket at all.
fn clear_stale(socket: &Path) -> Result<(), ClaimError> {
    let metadata = match fs::symlink_metadata(socket) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(ClaimError::Io {
                what: "inspect",
                path: socket.to_path_buf(),
                error,
            });
        }
    };
    if !metadata.file_type().is_socket() {
        return Err(ClaimError::NotASocket {
            socket: socket.to_path_buf(),
        });
    }
    if answers(socket)? {
        return Err(ClaimError::Answering {
            socket: socket.to_path_buf(),
        });
    }
    match fs::remove_file(socket) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ClaimError::Io {
            what: "remove the stale socket",
            path: socket.to_path_buf(),
            error,
        }),
    }
}
