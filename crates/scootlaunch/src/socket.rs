//! The single-instance toggle: one panel at a time, on one bind.
//!
//! While the panel is open it listens on
//! `$XDG_RUNTIME_DIR/scootlaunch-<WAYLAND_DISPLAY>.sock`. A second
//! invocation connects, sends `close`, and exits: one bind toggles. A
//! failed connect (or a stale file) means none is open, so the newcomer
//! unlinks the file and listens itself. There is no pid file (no pid-reuse
//! window) and no abstract name (any local user could bind one first:
//! `$XDG_RUNTIME_DIR` is the user's own `0700` directory, which rules that
//! out). The listener checks the peer's uid through `SO_PEERCRED` and
//! ignores any other, as scoot's own IPC does.
//!
//! The socket exists only while the panel is open, and is removed on
//! close: [`Claim`] removes the file when dropped, on every exit path.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

/// What the toggle says. A line, so a reader can frame it; the listener
/// answers nothing.
const CLOSE_LINE: &[u8] = b"close\n";
/// How long the second invocation waits on a live but stuck listener
/// before giving up on telling it (it still exits: the toggle was asked).
const PEER_TIMEOUT: Duration = Duration::from_secs(2);

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

/// Opening the toggle: either the panel listens now, or a running panel
/// was told to close.
pub enum Open {
    /// No panel was open: listen on the socket while open.
    Listen(Claim),
    /// A running panel was sent `close`: exit without opening.
    Signalled,
}

/// Opens the toggle at `path`: connect first (a live panel is told to
/// close), else unlink a stale file and listen.
pub fn open(path: &std::path::Path) -> Result<Open, String> {
    match UnixStream::connect(path) {
        Ok(stream) => {
            // A connectable socket is a live panel: tell it to close. A
            // write failure means it is dying; the toggle was still
            // asked, so exit either way.
            let _ = stream.set_write_timeout(Some(PEER_TIMEOUT));
            let _ = (&stream).write_all(CLOSE_LINE);
            Ok(Open::Signalled)
        }
        Err(_) => {
            // No listener (or a stale file): take the name over.
            std::fs::remove_file(path).ok();
            let listener = UnixListener::bind(path)
                .map_err(|error| format!("cannot listen on {}: {error}", path.display()))?;
            listener
                .set_nonblocking(true)
                .map_err(|error| format!("cannot poll {}: {error}", path.display()))?;
            Ok(Open::Listen(Claim {
                listener,
                path: path.to_owned(),
            }))
        }
    }
}

/// The panel's listen: the bound socket plus its path. Dropping removes
/// the file, so no exit path leaves a stale socket behind.
#[derive(Debug)]
pub struct Claim {
    listener: UnixListener,
    path: PathBuf,
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
    }
}

impl std::os::unix::io::AsFd for Claim {
    fn as_fd(&self) -> std::os::unix::io::BorrowedFd<'_> {
        self.listener.as_fd()
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
