//! The Wayland accept loop: draining the listening socket without dying.
//!
//! This replaces Smithay's [`ListeningSocketSource`] with the same shape and
//! one load-bearing difference: what an `accept` error does. Smithay's source
//! drains with `while let Some(client) = socket.accept()?` (`socket.rs:114-121`
//! at the pinned rev), so any error but `WouldBlock` -- which
//! [`ListeningSocket::accept`] already folds to `Ok(None)` (`socket.rs:144-150`
//! in wayland-server 0.31.14) -- propagates as `Err` out of `process_events`,
//! through calloop's `dispatch_events` (`loop_logic.rs:527`, `?`), out of
//! `EventLoop::run` (`loop_logic.rs:657+`, `?`), out of `compositor::run`
//! (`mod.rs:193`, `?`), and `main` prints it and exits `FAILURE`. An `EMFILE`
//! on this socket does not take the socket down. It takes the whole
//! compositor down -- every client's unsaved state with it -- which is the
//! crash/hang severity class, and exactly what the per-connection fd bounds
//! make reachable: two connections at the 512-buffer cap hold ~2 x 513 fds
//! against a 1024-fd table (see `wl_buffers.rs`), each connection inside
//! every bound, nothing tripped, and the next backlog entry kills the
//! process. Found re-deriving the connection-cap ticket's arithmetic against
//! the current code; see `docs/backlog/resolved/wayland-connection-cap-done.md`.
//!
//! So this drains the way [`ipc::accept`] does: every error maps to a
//! [`PostAction`], and none ever propagates. `EMFILE`/`ENFILE` sheds one
//! pending connection per turn through the shared [`Spare`], a dead listener
//! deregisters, anything else logs loudly and leaves the listener registered.
//! What a shed client sees is an immediate EOF with no refusal line -- there
//! is no protocol channel for refusing a Wayland connection gracefully (the
//! ticket's own point), and no fd to serve even an out-of-band reason with,
//! the same wire shape as the IPC shed.
//!
//! Deliberately not a connection-count cap: any cap a real session fits
//! through (bar, panels, launcher, apps, dialogs -- dozens) still admits the
//! two greedy connections that fill the table, so a count would deny shells
//! on a miscount while stopping nothing. The record states the arithmetic.
//!
//! ## What may run in a forked child
//!
//! Same discipline as [`ipc::accept`]: the exhaustion test lowers
//! `RLIMIT_NOFILE` in a forked child, so [`drain`], [`shed_one`] and
//! everything they call must stay allocation- and lock-free. That is why the
//! shed-accepted socket *becomes* the new spare (via [`Spare::put`]) instead
//! of being dropped and reopened, and why the backlog-empty re-arm is a raw
//! `libc::open` on a static path, never `File::open`. Keep it that way.
//!
//! [`ListeningSocketSource`]: smithay::wayland::socket::ListeningSocketSource
//! [`ListeningSocket::accept`]: smithay::reexports::wayland_server::ListeningSocket::accept
//! [`ipc::accept`]: super::ipc::accept
//! [`Spare`]: super::ipc::accept::Spare
//! [`Spare::put`]: super::ipc::accept::Spare::put

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{
    EventSource, Interest, Mode, Poll, PostAction, Readiness, Token, TokenFactory,
};
use smithay::reexports::wayland_server::{BindError, ListeningSocket};

use super::fd_pressure::Table;
use super::ipc::accept::{Disposition, ShedOutcome, Spare, classify};
use super::state::{ClientState, State};

#[cfg(test)]
mod tests;

/// A Wayland listening socket event source that survives `accept` errors.
///
/// [`Generic`] over the [`ListeningSocket`] gives registration for free; the
/// difference from Smithay's source is all in `process_events`, which drains
/// through [`drain`] below instead of propagating the first error. Owns its
/// [`Spare`]: one fd held for the session, the same cost as the IPC loop's.
/// (No `Debug`: [`Spare`] has none, and nothing requires it of a source.)
pub(super) struct WaylandListener {
    socket: Generic<ListeningSocket>,
    spare: Spare,
    /// The name bound in `$XDG_RUNTIME_DIR`, e.g. `wayland-2`.
    ///
    /// Stored (rather than read back out of the [`ListeningSocket`]) because
    /// binding goes through [`ListeningSocket::bind_absolute`], which leaves
    /// `socket_name()` as `None`: the name is a property of which candidate
    /// this listener claimed, not of the socket object.
    name: OsString,
}

impl WaylandListener {
    /// Binds `wayland-1` through `wayland-32`, like Smithay's
    /// `ListeningSocketSource::new_auto` (which this replaces at the one call
    /// site): `wayland-0` is skipped because clients may connect to the wrong
    /// compositor, and everything downstream reads the name out of the
    /// environment this process sets after `State::new`.
    ///
    /// Never takes the host compositor's own socket (see
    /// [`host_excluded_name`]): under `--nested` that name is the one
    /// `nested::init` must connect to, and binding it unlinks the live host
    /// socket -- wayland-server unlinks an existing file once it holds the
    /// `.lock`, which a host that never takes an exclusive `flock` (pixelflux
    /// in webtop) leaves free -- and then the session talks to itself. A live
    /// socket is never taken on lock evidence alone either (see
    /// [`socket_is_live`]), in any mode.
    pub(super) fn bind_auto() -> Result<Self, BindError> {
        Self::bind_auto_excluding(Self::host_excluded_name().as_deref())
    }

    /// The `$WAYLAND_DISPLAY` name that must not be bound, if it names a
    /// socket in the same `$XDG_RUNTIME_DIR` this process binds in.
    ///
    /// A bare name (`wayland-1`) is what `connect_to_env` resolves against
    /// `$XDG_RUNTIME_DIR`, so it collides directly. An absolute path only
    /// collides when its parent is the runtime dir; otherwise the host socket
    /// lives elsewhere and no candidate collides with it. Anything else
    /// (unset display, a name with no file-name part): nothing to exclude,
    /// and binding then fails with the runtime-dir error as before.
    pub(super) fn host_excluded_name() -> Option<OsString> {
        excluded_name_for(
            std::env::var_os("WAYLAND_DISPLAY").as_deref(),
            std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
        )
    }

    /// [`bind_auto`] over an explicit exclusion, so tests can drive the bind
    /// without touching the process-global environment.
    pub(super) fn bind_auto_excluding(exclude: Option<&OsStr>) -> Result<Self, BindError> {
        let runtime: PathBuf = std::env::var("XDG_RUNTIME_DIR")
            .map_err(|_| BindError::RuntimeDirNotSet)?
            .into();
        if !runtime.is_absolute() {
            return Err(BindError::RuntimeDirNotSet);
        }
        Self::bind_in(&runtime, exclude)
    }

    /// Binds the first `wayland-1..32` in `runtime_dir` that is neither
    /// excluded nor live.
    ///
    /// The flock is checked *before* probing, so a sibling scoot instance
    /// (which always holds its `<name>.lock` while bound) is never even
    /// connected to: no transient zero-byte client is injected into a running
    /// fixture sharing this runtime dir. Only lock-free names are probed, so
    /// a server that does not share the flock scheme (pixelflux in webtop
    /// holds no lock) still keeps its socket: only a stale file (or no file)
    /// reaches [`ListeningSocket::bind_absolute`], whose `Drop` then removes
    /// only what this session bound. `AlreadyInUse` (a racing second scoot
    /// won the lock between this check and the bind) moves to the next name;
    /// anything else fails the bind, as `bind_auto` always did.
    ///
    /// Residual TOCTOU (inherent): lock-check -> probe -> `bind_absolute`
    /// re-lock is not atomic. A lock-free host binding a probed-free name in
    /// that microsecond window still loses its socket to the unlink in
    /// `bind_absolute`. The `$WAYLAND_DISPLAY` exclusion closes this for the
    /// nested case (the only shape where the colliding server is known ahead
    /// of time); the probe closes it for every steady-state live socket and
    /// narrows the window to a concurrent bind racing startup.
    fn bind_in(runtime_dir: &Path, exclude: Option<&OsStr>) -> Result<Self, BindError> {
        for index in 1..33 {
            let name = format!("wayland-{index}");
            if exclude.is_some_and(|excluded| excluded == OsStr::new(&name)) {
                continue;
            }
            let path = runtime_dir.join(&name);
            // Flock first: a held lock means a sibling scoot owns this name.
            // Skip with no connect, so parallel suites sharing one runtime
            // dir never see a probe client. Permission-denied (or any other
            // lock-open failure but missing file) skips conservatively, the
            // same orientation as the probe's conservative-live arm.
            if lock_blocks_probe(&path.with_extension("lock")) {
                continue;
            }
            if socket_is_live(&path) {
                continue;
            }
            match ListeningSocket::bind_absolute(path) {
                Ok(socket) => {
                    return Ok(Self {
                        socket: Generic::new(socket, Interest::READ, Mode::Level),
                        spare: Spare::new(),
                        name: OsString::from(name),
                    });
                }
                Err(BindError::AlreadyInUse) => continue,
                Err(BindError::RuntimeDirNotSet) => return Err(BindError::RuntimeDirNotSet),
                Err(other) => return Err(other),
            }
        }
        Err(BindError::AlreadyInUse)
    }

    /// The name clients connect to, e.g. `wayland-1`: the candidate
    /// [`bind_in`] claimed.
    pub(super) fn socket_name(&self) -> OsString {
        self.name.clone()
    }
}

/// Which `$WAYLAND_DISPLAY` value excludes a bind candidate, if any.
///
/// Pure (both inputs passed in) so tests can pin every shape without touching
/// the process-global environment, which `State::new`'s own tests cannot do
/// (see `state::tests`).
fn excluded_name_for(display: Option<&OsStr>, runtime_dir: Option<&OsStr>) -> Option<OsString> {
    let display_path = Path::new(display?);
    let file = display_path.file_name()?;
    if display_path.is_absolute() {
        // Only a host socket in the same runtime dir collides with a
        // candidate; anything elsewhere binds nowhere near `bind_in`'s names.
        if display_path.parent() != runtime_dir.map(Path::new) {
            return None;
        }
    } else if display_path.components().count() != 1 {
        // Relative with a separator: resolved against the cwd, never against
        // the runtime dir, so no candidate collides with it.
        return None;
    }
    // Bare name: `connect_to_env` resolves it against `$XDG_RUNTIME_DIR`.
    Some(file.to_os_string())
}

/// Whether the lock file at `lock_path` is held or otherwise not provably
/// free, in which case the candidate must be skipped with no `connect`.
///
/// Mirrors the open/flock/dev-ino check in `ListeningSocket::bind_absolute`
/// (`socket.rs` at the pinned wayland-server rev): open the existing file
/// (missing file = free, anything else that fails to open = conservative
/// skip), try `LOCK_EX|LOCK_NB` (contended = skip), then verify the locked
/// fd is still the file on disk (unlinked-between-open-and-flock = retry,
/// exactly as `bind_absolute` does). A lock proved free is released on
/// return -- `bind_absolute` re-locks as today -- so a racing sibling that
/// wins between here and the bind still surfaces as `AlreadyInUse`.
fn lock_blocks_probe(lock_path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    loop {
        let lock = match File::open(lock_path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return false,
            Err(_) => return true,
        };
        // SAFETY: `flock` on an open fd is always safe; the fd is valid.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return true;
        }
        let fd_meta = match lock.metadata() {
            Ok(meta) => meta,
            Err(_) => return true,
        };
        let disk_meta = match std::fs::metadata(lock_path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => return true,
        };
        if fd_meta.dev() == disk_meta.dev() && fd_meta.ino() == disk_meta.ino() {
            return false;
        }
        // Locked a handle since unlinked: retry, as `bind_absolute` does.
    }
}

/// How long one liveness probe may block startup waiting for a sibling
/// backlog that never drains. Local `connect` answers immediately; this only
/// fires against a listener that stopped accepting, and then the name counts
/// as live rather than hanging the session.
const PROBE_TIMEOUT: Duration = Duration::from_millis(100);

/// Whether anything is listening at `path`: a successful `connect` means a
/// live server owns it, whatever the lock files say.
///
/// Only `ConnectionRefused` (a socket file with no listener: stale) and
/// `NotFound` (no file at all) count as free. Any other failure is treated as
/// live -- a socket this process may not connect to (permissions, a racing
/// unlink, a full backlog, a connect that times out) is someone else's
/// either way, and unlinking it to find out would be the bug this probe
/// exists to prevent.
///
/// Nonblocking with a short timeout: a blocking `connect` against a sibling
/// whose backlog is full (128 pending) would hang startup with no message,
/// where the old code at least failed loudly-ish.
fn socket_is_live(path: &Path) -> bool {
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() >= 108 {
        return true;
    }
    // SAFETY: `socket` with these args either returns an owned fd or -1.
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return true;
    }
    // Set nonblocking so a full backlog surfaces as `EINPROGRESS` + `poll`
    // instead of hanging startup.
    // SAFETY: `fcntl` on an owned, valid fd is safe.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0 {
        unsafe { libc::close(fd) };
        return true;
    }
    // SAFETY: zeroed `sockaddr_un` with `sun_family` set and the path bytes
    // copied into `sun_path`; length covers family + bytes + nul.
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (slot, byte) in addr.sun_path.iter_mut().zip(bytes.iter()) {
        *slot = *byte as libc::c_char;
    }
    let addr_len = (std::mem::size_of::<libc::sa_family_t>() + bytes.len() + 1) as libc::socklen_t;
    // SAFETY: `addr` is a valid `sockaddr_un` of `addr_len` bytes; `fd` is a
    // unix-stream fd owned above and closed on every return below.
    let connected =
        unsafe { libc::connect(fd, (&addr as *const libc::sockaddr_un).cast(), addr_len) };
    if connected == 0 {
        unsafe { libc::close(fd) };
        return true;
    }
    // SAFETY: `__errno_location` is thread-local and valid to read.
    let errno = unsafe { *libc::__errno_location() };
    match errno {
        libc::ECONNREFUSED | libc::ENOENT => {
            unsafe { libc::close(fd) };
            false
        }
        libc::EAGAIN => {
            // Full backlog on a nonblocking unix `connect`: live, not free.
            unsafe { libc::close(fd) };
            true
        }
        libc::EINPROGRESS => {
            let mut waited = Duration::ZERO;
            let step = Duration::from_millis(10);
            let result = loop {
                let mut pfd = libc::pollfd {
                    fd,
                    events: libc::POLLOUT,
                    revents: 0,
                };
                // SAFETY: `pfd` is a valid one-entry poll set; timeout is small.
                let ready = unsafe { libc::poll(&mut pfd, 1, 10) };
                if ready > 0 {
                    let mut so_error: libc::c_int = 0;
                    let mut option_len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
                    // SAFETY: `getsockopt` writes one `c_int` into `so_error`.
                    let got = unsafe {
                        libc::getsockopt(
                            fd,
                            libc::SOL_SOCKET,
                            libc::SO_ERROR,
                            (&mut so_error as *mut libc::c_int).cast(),
                            &mut option_len,
                        )
                    };
                    if got != 0 {
                        break true;
                    }
                    match so_error {
                        0 => break true,
                        libc::ECONNREFUSED | libc::ENOENT => break false,
                        _ => break true,
                    }
                } else if ready == 0 {
                    waited += step;
                    if waited >= PROBE_TIMEOUT {
                        break true;
                    }
                } else {
                    // SAFETY: errno read as above.
                    let poll_errno = unsafe { *libc::__errno_location() };
                    if poll_errno == libc::EINTR {
                        continue;
                    }
                    break true;
                }
            };
            unsafe { libc::close(fd) };
            result
        }
        _ => {
            unsafe { libc::close(fd) };
            true
        }
    }
}

impl EventSource for WaylandListener {
    type Event = UnixStream;
    type Metadata = ();
    type Ret = ();
    type Error = io::Error;

    fn process_events<F>(
        &mut self,
        readiness: Readiness,
        token: Token,
        mut callback: F,
    ) -> io::Result<PostAction>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        self.socket.process_events(readiness, token, |_, socket| {
            Ok(drain(socket, &self.spare, &mut |stream| {
                callback(stream, &mut ())
            }))
        })
    }

    fn register(
        &mut self,
        poll: &mut Poll,
        token_factory: &mut TokenFactory,
    ) -> smithay::reexports::calloop::Result<()> {
        self.socket.register(poll, token_factory)
    }

    fn reregister(
        &mut self,
        poll: &mut Poll,
        token_factory: &mut TokenFactory,
    ) -> smithay::reexports::calloop::Result<()> {
        self.socket.reregister(poll, token_factory)
    }

    fn unregister(&mut self, poll: &mut Poll) -> smithay::reexports::calloop::Result<()> {
        self.socket.unregister(poll)
    }
}

/// Takes one accepted stream into the compositor, or sheds it under
/// compositor-wide fd pressure.
///
/// A shed stream is dropped: the client sees an immediate EOF, the same
/// wire shape as the `EMFILE` shed in [`drain`] -- there is no protocol
/// channel for refusing a Wayland connection gracefully, and serving it
/// would spend fds the table does not have. An unknown table (`None`)
/// admits: shedding on a broken gauge would deny innocents for a faulty
/// observation, and the `EMFILE` shed still catches real exhaustion
/// underneath. No double-shed with [`drain`]: the two are mutually
/// exclusive per backlog entry (an entry is either accepted, reaching
/// here, or its `accept` fails, reaching the shed arm), and this path
/// never touches the spare, so a pressure that deepens into exhaustion
/// still finds it armed.
///
/// Takes the observed `table` rather than reading it so tests can drive
/// every shape with a canned reading; the one production caller (the
/// `listen` callback in `state.rs`) passes `fd_pressure::table()`, which is
/// cached: a whole backlog drained in one callback costs one observation.
/// Allocation is fine here: this runs on the loop thread in the accept
/// callback, never in a forked exhaustion-test child (which drives
/// [`drain`] directly with its own stub).
pub(super) fn admit(state: &mut State, stream: UnixStream, table: Option<Table>) {
    if let Some(observed) = table.filter(|observed| observed.pressured()) {
        tracing::warn!(
            used = observed.used,
            soft = observed.soft,
            free = observed.free(),
            "out of file descriptors; shed a pending wayland connection \
             (fd pressure, not a cap: there is no protocol channel for \
             refusing a wayland connection, so a shed one gets EOF)"
        );
        return;
    }
    // A new connection can fail under fd/id exhaustion; that's the
    // misbehaving-client's problem; a single bad file descriptor
    // shouldn't take down every other client's session.
    match state
        .display_handle
        .insert_client(stream, Arc::new(ClientState::default()))
    {
        // Counted against the cached table reading, so the rest of this
        // burst is judged against a count that includes this connection
        // (see `fd_pressure::table`).
        Ok(_) => super::fd_pressure::note_opened(1),
        Err(error) => tracing::warn!(%error, "could not accept a new wayland client"),
    }
}

/// Spends the spare to consume one backlog entry: close it, accept the pending
/// connection, and keep that connection's fd as the new spare.
///
/// Same shape as [`ipc::accept`]'s `shed_one`, over [`ListeningSocket`]
/// instead of `UnixListener` (whose `accept` yields an address this one does
/// not, and whose `WouldBlock` arrives as `Ok(None)` rather than `Err`).
/// Same fork-child discipline: no allocation, no locks -- the accepted socket
/// becomes the spare via [`Spare::put`], and only the backlog-empty path
/// reopens, as a raw `libc::open` on a static path.
fn shed_one(socket: &ListeningSocket, spare: &Spare) -> ShedOutcome {
    // The whole trick, as in `ipc::accept`: this close frees exactly one fd,
    // which is what the accept below spends. The table is process-global, so
    // another thread can still steal it in the window (since the screenshot
    // worker, anything doing `open`); narrow and self-resolving, but the shape
    // the `Stuck` corner has.
    drop(spare.take());
    match socket.accept() {
        Ok(Some(pending)) => {
            spare.put(File::from(OwnedFd::from(pending)));
            ShedOutcome::Consumed
        }
        // Nothing pending after all (or a signal got in first): the earlier
        // error raced a client going away. Re-arm the spent spare straight
        // away -- raw `open`, not `File::open`: see above -- and stay quiet.
        // (`ListeningSocket::accept` reports an empty backlog as `Ok(None)`;
        // `EINTR` arrives as `Err`, matched below.)
        Ok(None) => {
            re_arm(spare);
            ShedOutcome::BacklogEmpty
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            re_arm(spare);
            ShedOutcome::BacklogEmpty
        }
        Err(error) => ShedOutcome::Stuck(error),
    }
}

/// Re-arms the spare after a shed found no backlog, exactly like
/// [`ipc::accept`]'s: a raw `libc::open`, never `File::open`.
fn re_arm(spare: &Spare) {
    let fd = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd >= 0 {
        // SAFETY: `open` just returned this fd; it is open and owned.
        spare.put(unsafe { File::from_raw_fd(fd) });
    } else {
        // The freed fd was stolen in the window (or `/dev/null` would not
        // open): disarmed, loudly rather than silently.
        tracing::warn!(
            "could not re-arm the wayland accept loop's spare fd; \
             further fd exhaustion will be logged but not shed"
        );
    }
}

/// Accepts until the backlog is empty, shedding through the spare on fd
/// exhaustion. Returns what the event-loop callback should return with:
///
/// - `Continue`: the backlog is drained (or was never the problem), and the
///   level trigger has nothing left to re-report -- the no-spin guarantee.
/// - `Remove`: the listener itself is dead; waking into the same failure
///   forever would be the spin this module exists to prevent.
///
/// Never `Err`: that propagation is the compositor-killing bug this module
/// replaces (see the module doc). The per-connection work -- inserting the
/// client, which can itself fail under id exhaustion -- stays the caller's:
/// `take` receives every accepted stream, and what it returns is not observed
/// here.
fn drain(socket: &ListeningSocket, spare: &Spare, take: &mut impl FnMut(UnixStream)) -> PostAction {
    loop {
        match socket.accept() {
            Ok(Some(stream)) => take(stream),
            Ok(None) => return PostAction::Continue,
            Err(error) => match classify(&error) {
                Disposition::Done => return PostAction::Continue,
                Disposition::Shed => match shed_one(socket, spare) {
                    ShedOutcome::Consumed => {
                        tracing::warn!(
                            "out of file descriptors; shed a pending wayland connection \
                             (exhaustion, not a cap: there is no protocol channel for \
                             refusing a wayland connection, so a shed one gets EOF)"
                        );
                    }
                    ShedOutcome::BacklogEmpty => return PostAction::Continue,
                    ShedOutcome::Stuck(error) => {
                        tracing::error!(
                            %error,
                            "wayland accept keeps failing with no fd to spend; \
                             leaving the backlog pending"
                        );
                        return PostAction::Continue;
                    }
                },
                Disposition::Dead => {
                    tracing::error!(%error, "wayland listener is dead; removing the accept source");
                    return PostAction::Remove;
                }
                Disposition::Other => {
                    tracing::error!(%error, "wayland accept failed; leaving the listener registered");
                    return PostAction::Continue;
                }
            },
        }
    }
}
