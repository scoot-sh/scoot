//! The single-instance toggle: one panel at a time, on one bind.
//!
//! While the panel is open it listens on
//! `$XDG_RUNTIME_DIR/scootlaunch-<WAYLAND_DISPLAY>.sock`. A second
//! invocation connects, sends `close`, and exits: one bind toggles.
//! Becoming the listener is atomic: a lock file beside the socket
//! (`scootlaunch-<display>.lock`, an exclusive nonblocking `flock`
//! held for the panel's life) serialises the connect-or-bind, so two
//! opens in the same instant yield one panel instead of orphaning one
//! (both failing the first connect, both unlinking, both binding,
//! the loser's unlink deleting the winner's file while its listener
//! still holds the unlinked inode). The lock, not the socket, says
//! whether an opener is in progress: the kernel drops it when the
//! process dies, however it dies, so a crash leaves no stale claim.
//! The listener checks the peer's uid through `SO_PEERCRED` and
//! ignores any other, as scoot's own IPC does.
//!
//! The socket exists only while the panel is open, and is removed on
//! close: [`Claim`] removes the file when dropped, on every exit path.
//! It is published `0600` with no umask-dependent window: built inside
//! a freshly-created `0700` staging directory beside the path, chmodded
//! before the atomic rename onto the published name (as scoot's IPC
//! listener does), so a client only ever finds the previous socket or
//! this `0600` one, never a half-built or more-open one. `chmod` after
//! bind would leave exactly that window (`0777 & !umask`, e.g. `0755`
//! under `022`, connectable until the chmod lands).

use std::io::Read;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rustix::fs::{FlockOperation, flock};
use rustix::io::Errno;

/// What the toggle says. A line, so a reader can frame it; the listener
/// answers nothing.
const CLOSE_LINE: &[u8] = b"close\n";
/// How long the second invocation waits on a live but stuck listener
/// before giving up on telling it (it still exits: the toggle was asked).
const PEER_TIMEOUT: Duration = Duration::from_secs(2);
/// `sun_path` holds 108 bytes including its NUL: 107 is the longest path
/// a unix socket can bind at.
const SUN_PATH_MAX: usize = 107;
/// The socket is 0600 before it is reachable: owner read/write.
const SOCKET_MODE: u32 = 0o600;
/// The staging directory is 0700: owner only, so nobody else can list it.
const STAGING_MODE: u32 = 0o700;
/// How many staging names to try before giving up.
const STAGING_ATTEMPTS: usize = 16;

/// The socket's filename for `display`: `scootlaunch-<display>.sock`. The
/// display goes through the value grammar (a newline, `/` or control
/// character in it would escape the runtime dir), so anything outside
/// `[A-Za-z0-9._-]{1,64}` is refused rather than sanitized.
fn file_name(display: &std::ffi::OsStr) -> Result<String, String> {
    let bytes = display.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return Err(format!(
            "a display name is 1 to 64 bytes, not {}",
            bytes.len()
        ));
    }
    let ok = bytes.iter().all(|byte| {
        byte.is_ascii_alphanumeric() || *byte == b'.' || *byte == b'_' || *byte == b'-'
    });
    if !ok {
        return Err(format!(
            "display name {display:?} is not a socket name; refusing to build a socket path from it"
        ));
    }
    Ok(format!(
        "scootlaunch-{}.sock",
        display.to_str().unwrap_or_default()
    ))
}

use std::os::unix::ffi::OsStrExt as _;

/// Resolves the socket path from the live environment: `$XDG_RUNTIME_DIR`
/// holding `scootlaunch-<WAYLAND_DISPLAY>.sock`. Pure over its inputs, so
/// tests never touch the process environment.
pub fn socket_path(
    runtime: Option<&std::ffi::OsStr>,
    display: Option<&std::ffi::OsStr>,
) -> Result<PathBuf, String> {
    let Some(runtime) = runtime.filter(|dir| !dir.is_empty()) else {
        return Err(String::from(
            "no XDG_RUNTIME_DIR: the panel needs a Wayland session",
        ));
    };
    let Some(display) = display.filter(|name| !name.is_empty()) else {
        return Err(String::from(
            "no WAYLAND_DISPLAY: the panel needs a Wayland session",
        ));
    };
    Ok(PathBuf::from(runtime).join(file_name(display)?))
}

/// [`socket_path`] over the live environment, for the panel's open path.
pub fn live_socket_path() -> Result<PathBuf, String> {
    socket_path(
        std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
    )
}

/// The lock beside `socket`: the same name with the `.sock` suffix
/// replaced by `.lock`, so one display's claim never collides with
/// another's, and the lock's lifetime matches the socket's.
fn lock_path_for(socket: &Path) -> PathBuf {
    if socket.extension().is_some_and(|ext| ext == "sock") {
        socket.with_extension("lock")
    } else {
        let mut lock = socket.as_os_str().to_owned();
        lock.push(".lock");
        PathBuf::from(lock)
    }
}

/// Opening the toggle: either the panel listens now, or a running panel
/// was told to close.
pub enum Open {
    /// No panel was open: listen on the socket while open.
    Listen(Claim),
    /// A running panel was sent `close` (or another opener is opening,
    /// so yielding leaves one panel): exit without opening.
    Signalled,
}

/// Opens the toggle at `path`: connect first (a live panel is told to
/// close), else claim the single instance under the lock and listen.
///
/// Two opens at once serialise on the lock: the winner binds and listens
/// while holding it, the loser waits briefly for the winner's panel and
/// yields without opening, so one panel results instead of two with the
/// first orphaned (its file unlinked from under its listener). A lock held
/// with no panel answering is a visible error, not a silent yield. The
/// socket is published `0600` through a `0700` staging directory before the
/// rename, never chmodded after bind.
pub fn open(path: &std::path::Path) -> Result<Open, String> {
    check_path_len(path)?;
    if try_signal(path)? {
        return Ok(Open::Signalled);
    }
    claim(path)
}

/// The toggle socket path must fit in `sun_path`: checked before any probe
/// so an overlong path gets the friendly message rather than the probe's
/// `ENAMETOOLONG`.
fn check_path_len(path: &Path) -> Result<(), String> {
    if path.as_os_str().len() > SUN_PATH_MAX {
        return Err(format!(
            "the socket path is {} bytes; a unix socket path can be at most {SUN_PATH_MAX}: {}",
            path.as_os_str().len(),
            path.display()
        ));
    }
    Ok(())
}

/// Whether a panel is listening, told without blocking: a nonblocking
/// connect that completes (or reports a full backlog) counts as listening.
/// Sends `close` when one is, without waiting past [`PEER_TIMEOUT`]: a
/// `SIGSTOP`ped panel with a full backlog must not hang the press. A write
/// failure means the panel is dying; the toggle was still asked, so either
/// way a listening panel reports `true`.
fn try_signal(path: &Path) -> Result<bool, String> {
    use rustix::net::{
        AddressFamily, SendFlags, SocketAddrUnix, SocketFlags, SocketType, connect, send,
        socket_with,
    };
    let probe = socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    )
    .map_err(|error| format!("cannot probe {}: {error}", path.display()))?;
    let address = SocketAddrUnix::new(path)
        .map_err(|error| format!("cannot probe {}: {error}", path.display()))?;
    match connect(&probe, &address) {
        Err(Errno::NOENT) | Err(Errno::CONNREFUSED) => return Ok(false),
        Ok(()) | Err(Errno::AGAIN) | Err(Errno::INPROGRESS) => {}
        Err(errno) => {
            return Err(format!("cannot probe {}: {errno}", path.display()));
        }
    }
    // Listening: wait for the connect to finish, then offer `close`.
    // A deaf panel (stopped, backlog full) times out here rather than
    // hanging the press; it still counts as signalled.
    let wait = rustix::time::Timespec {
        tv_sec: PEER_TIMEOUT.as_secs() as _,
        tv_nsec: PEER_TIMEOUT.subsec_nanos() as _,
    };
    let mut polls = [rustix::event::PollFd::new(
        &probe,
        rustix::event::PollFlags::OUT,
    )];
    let _ = rustix::event::poll(&mut polls, Some(&wait));
    let _ = send(&probe, CLOSE_LINE, SendFlags::empty());
    Ok(true)
}

/// After losing the lock race: waits briefly for the winner's panel, so a
/// normal simultaneous open still yields one panel, while a lock held with
/// no panel (a same-uid squatter, no panel) becomes a visible error.
fn wait_for_panel(path: &Path) -> Result<bool, String> {
    let deadline = std::time::Instant::now() + PEER_TIMEOUT;
    loop {
        if answers(path).map_err(|error| format!("cannot probe {}: {error}", path.display()))? {
            return Ok(true);
        }
        if std::time::Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// The slow path: no live panel answered, so become the one listener
/// under the lock.
fn claim(path: &Path) -> Result<Open, String> {
    check_path_len(path)?;
    let lock_path = lock_path_for(path);
    let lock = open_lock(&lock_path)?;
    match flock(&lock, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => {}
        Err(Errno::WOULDBLOCK) => {
            // Another opener holds the lock: it saw no panel either, so
            // it is opening now. Wait briefly for its panel, then yield
            // without signalling: the panel that wins opens, and one
            // panel results. Signalling here would close the panel that
            // just won. A lock that never produces a panel is a wedge:
            // report it loudly rather than exiting 0 with no panel.
            if wait_for_panel(path)? {
                return Ok(Open::Signalled);
            }
            return Err(format!(
                "another opener holds {} but no panel answered within {}s; refusing to yield silently",
                lock_path.display(),
                PEER_TIMEOUT.as_secs()
            ));
        }
        Err(errno) => {
            return Err(format!("cannot lock {}: {errno}", lock_path.display()));
        }
    }
    // Holding the lock: a socket that still answers belongs to a panel
    // running without this lock (its lock file was deleted under it).
    // Signal it rather than stealing its name.
    if answers(path).map_err(|error| format!("cannot probe {}: {error}", path.display()))? {
        let _ = try_signal(path);
        return Ok(Open::Signalled);
    }
    clear_stale(path)?;
    let listener = bind_staged(path)?;
    Ok(Open::Listen(Claim {
        listener,
        path: path.to_owned(),
        _lock: lock,
    }))
}

/// Opens the lock without following a symlink: `O_NOFOLLOW` refuses a
/// symlinked lock path rather than locking through it. No unlink or
/// truncate passes through it, but the care matches the staging pin
/// below.
fn open_lock(lock_path: &Path) -> Result<std::fs::File, String> {
    use rustix::fs::{Mode, OFlags};
    let fd = rustix::fs::open(
        lock_path,
        OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_bits_truncate(0o600),
    )
    .map_err(|error| format!("cannot open {}: {error}", lock_path.display()))?;
    Ok(std::fs::File::from(fd))
}

/// Whether something is listening on `socket`, without blocking: a
/// blocking connect would wait for room in the backlog of a listener
/// that has stopped, hanging start-up. `EAGAIN` (backlog full) counts as
/// listening; `ECONNREFUSED` and `ENOENT` as not.
fn answers(socket: &Path) -> Result<bool, rustix::io::Errno> {
    use rustix::net::{
        AddressFamily, SocketAddrUnix, SocketFlags, SocketType, connect, socket_with,
    };
    let probe = socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    )?;
    let address = SocketAddrUnix::new(socket)?;
    match connect(&probe, &address) {
        Ok(()) | Err(Errno::AGAIN) | Err(Errno::INPROGRESS) => Ok(true),
        Err(Errno::CONNREFUSED) | Err(Errno::NOENT) => Ok(false),
        Err(errno) => Err(errno),
    }
}

/// With the lock held: verifies a dead daemon's socket, refusing if the
/// path is live or not a socket at all. Never removes anything: the
/// rename in [`stage`] replaces a stale socket atomically, so the name
/// always resolves during handover with no missing-name window. Never
/// removes a non-socket: unlinking whatever sits at the name would let
/// a planted symlink aim the removal.
fn clear_stale(socket: &Path) -> Result<(), String> {
    use std::os::unix::fs::FileTypeExt as _;
    let metadata = match std::fs::symlink_metadata(socket) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!("cannot inspect {}: {error}", socket.display()));
        }
    };
    if !metadata.file_type().is_socket() {
        return Err(format!(
            "{} exists and is not a socket; refusing to replace it",
            socket.display()
        ));
    }
    if answers(socket).map_err(|errno| format!("cannot probe {}: {errno}", socket.display()))? {
        return Err(format!(
            "something is already answering on {} although no opener holds its lock; refusing to replace it",
            socket.display()
        ));
    }
    Ok(())
}

/// Binds the listening socket at `path` through a `0700` staging directory
/// beside it, chmods it `0600`, and renames it onto `path` atomically, so
/// the published name is only ever the previous socket or this `0600` one.
fn bind_staged(path: &Path) -> Result<UnixListener, String> {
    use std::os::unix::fs::DirBuilderExt;
    if path.as_os_str().len() > SUN_PATH_MAX {
        return Err(format!(
            "the socket path is {} bytes; a unix socket path can be at most {SUN_PATH_MAX}: {}",
            path.as_os_str().len(),
            path.display()
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        format!(
            "the socket path {} names no directory to build it in",
            path.display()
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
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && attempts > 0 => {
                continue;
            }
            Err(error) => {
                return Err(format!(
                    "cannot stage the socket in {}: {error}",
                    staging.display()
                ));
            }
        }
    }
}

/// Binds inside `staging` and publishes at `path`, then removes `staging`
/// again whether that worked or not.
fn publish(staging: &Path, path: &Path) -> Result<UnixListener, String> {
    let result = stage(staging, path);
    let _ = std::fs::remove_dir(staging);
    result
}

/// Binds the socket inside `staging`, gives it [`SOCKET_MODE`] and renames
/// it onto `path`.
///
/// `staging` is a directory this process just created, but that claim is
/// rechecked: it is reopened `O_DIRECTORY | O_NOFOLLOW`, `fchmod`ed to
/// [`STAGING_MODE`] through the fd, and bound under `/proc/self/fd/<n>`,
/// as scoot's IPC listener does, so a swapped name or a pathological
/// umask cannot redirect the bind or the chmod. `directory` stays alive
/// through the rename: `staged` borrows its fd number, and reusing that
/// number after an early drop would silently operate on the wrong file.
fn stage(staging: &Path, path: &Path) -> Result<UnixListener, String> {
    use rustix::fs::{Mode, OFlags};
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::PermissionsExt as _;
    let directory = rustix::fs::open(
        staging,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| format!("cannot stage the socket in {}: {error}", staging.display()))?;
    rustix::fs::fchmod(&directory, Mode::from_bits_truncate(STAGING_MODE))
        .map_err(|error| format!("cannot stage the socket in {}: {error}", staging.display()))?;
    let staged = PathBuf::from(format!("/proc/self/fd/{}/s", directory.as_raw_fd()));
    let listener = UnixListener::bind(&staged)
        .map_err(|error| format!("cannot listen on {}: {error}", path.display()))?;
    if let Err(error) = (|| -> std::io::Result<()> {
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(SOCKET_MODE))?;
        listener.set_nonblocking(true)?;
        std::fs::rename(&staged, path)?;
        Ok(())
    })() {
        let _ = std::fs::remove_file(&staged);
        return Err(format!("cannot publish {}: {error}", path.display()));
    }
    Ok(listener)
}

/// A name for the staging directory: unpredictable rather than merely
/// unique (a pid alone is guessable in the shared runtime dir), from the
/// kernel's random pool, with the pid and a counter mixed in so a
/// `getrandom` failure still yields a unique name.
fn staging_name() -> PathBuf {
    use rustix::rand::GetRandomFlags;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut bytes = [0u8; 12];
    let mut filled = 0;
    while filled < bytes.len() {
        match rustix::rand::getrandom(&mut bytes[filled..], GetRandomFlags::empty()) {
            Ok(read) if read > 0 => filled += read,
            _ => break,
        }
    }
    let counter = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let pid = std::process::id();
    let mut name = format!(".scootlaunch-{pid}-{counter}-");
    name.extend(
        bytes[..filled]
            .iter()
            .map(|byte| char::from(ALPHABET[usize::from(*byte) % ALPHABET.len()])),
    );
    // If the kernel gave nothing, the pid+counter still uniquifies.
    if filled == 0 {
        name.push('x');
    }
    PathBuf::from(name)
}

/// The panel's listen: the bound socket plus its path, with the lock held
/// for the panel's life. Dropping removes the file first and then drops
/// the lock, so no exit path leaves a stale socket behind and a new panel
/// can start at once.
#[derive(Debug)]
pub struct Claim {
    listener: UnixListener,
    path: PathBuf,
    /// Held until drop: the open lock file keeping the `flock` for the
    /// panel's life.
    _lock: std::fs::File,
}

impl Claim {
    /// Drains pending connections: `true` when a same-uid peer said
    /// `close`. Never blocks (the listener is nonblocking; each peer gets
    /// a bounded read). Peers of another uid are ignored, as is garbage.
    pub fn accept_close(&self) -> bool {
        loop {
            let stream = match self.listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return false,
                Err(_) => return false,
            };
            if said_close(&stream) {
                return true;
            }
        }
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).ok();
        // `_lock` drops here, releasing the flock after the file is gone.
    }
}

impl std::os::unix::io::AsFd for Claim {
    fn as_fd(&self) -> std::os::unix::io::BorrowedFd<'_> {
        std::os::unix::io::AsFd::as_fd(&self.listener)
    }
}

/// Whether `stream` is a same-uid peer saying `close`: anything else
/// (another uid, garbage, a read failure) is ignored.
fn said_close(stream: &UnixStream) -> bool {
    if !same_uid(stream) {
        return false;
    }
    let _ = stream.set_read_timeout(Some(PEER_TIMEOUT));
    let mut buf = [0u8; 16];
    let mut text = Vec::new();
    let mut stream = stream;
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(read) => {
                text.extend_from_slice(&buf[..read]);
                if text.len() >= CLOSE_LINE.len() || text.contains(&b'\n') {
                    break;
                }
            }
            Err(_) => return false,
        }
    }
    text.starts_with(CLOSE_LINE)
}

/// Whether the peer runs as us: `SO_PEERCRED`'s uid against our euid. A
/// query failure is not "same": the peer is ignored.
#[cfg(target_os = "linux")]
fn same_uid(stream: &UnixStream) -> bool {
    use std::os::unix::io::AsFd as _;
    let peer = rustix::net::sockopt::socket_peercred(stream.as_fd());
    let ours = rustix::process::geteuid();
    peer.map(|cred| cred.uid == ours).unwrap_or(false)
}

/// No `SO_PEERCRED` off Linux: the panel itself is Linux-only (it speaks
/// layer-shell), so this path only serves the unit tests there.
#[cfg(not(target_os = "linux"))]
fn same_uid(_stream: &UnixStream) -> bool {
    true
}

#[cfg(test)]
mod tests;
