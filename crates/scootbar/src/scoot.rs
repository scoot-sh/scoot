//! `{ scoot = "quit" }`: a request to scoot's own control socket, without a
//! process spawn.
//!
//! **Hand-written, not `scoot-ipc`.** The request is one fixed line
//! (`{"type":"action","action":"quit"}`), so this writes it and reads the
//! one-line reply. The `scoot-ipc` client was measured first, as the
//! backlog entry asked: linking it (its `Request` and `Response` types
//! with their serde derives, and `base64`) costs **+131,072 bytes, 10%**
//! of the release binary, against about 2.5 KB for these forty lines
//! (`docs/scootbar/backlog/resolved/pointer-and-interactions-done.md` has
//! the numbers). The wire cannot drift unnoticed: a test encodes the same
//! request with `scoot-ipc`, a dev-dependency only, and compares.
//!
//! **It never blocks the bar's loop for long.** The connect is non-blocking
//! (a full accept queue is "scoot is busy", not a wait), and the request and
//! its reply are one short line each way on a fresh connection, with a
//! timeout on both and a bound on the reply: a wedged scoot costs the bar at
//! most [`TIMEOUT`] once, on a click. A scoot that quits before answering,
//! which is what `quit` does, closes the connection: that is success.
//!
//! Absent scoot (another compositor, or no session), the action says it
//! cannot run and does nothing.

use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rustix::net::{AddressFamily, SocketAddrUnix, SocketFlags, SocketType, connect, socket_with};

use crate::action::ScootAction;

#[cfg(test)]
mod tests;

/// The longest the bar waits on scoot, each way.
pub const TIMEOUT: Duration = Duration::from_millis(250);

/// The longest reply read: an answer to `quit` is a few bytes.
const MAX_REPLY: u64 = 4096;

/// Overrides the socket path: scoot sets it for what it starts
/// (`scoot_ipc::SOCKET_ENV`).
const SOCKET_ENV: &str = "SCOOT_SOCKET";
const SOCKET_NAME: &str = "scoot.sock";

/// The request line for `action`: what `scoot-ipc` encodes `Request::Action`
/// as (checked by a test).
fn request(action: ScootAction) -> &'static str {
    match action {
        ScootAction::Quit => "{\"type\":\"action\",\"action\":\"quit\"}\n",
    }
}

/// Where scoot's socket is: `SCOOT_SOCKET`, else `scoot.sock` in
/// `XDG_RUNTIME_DIR`; an empty value is unset (as `scoot-ipc` reads them).
fn resolve(explicit: Option<OsString>, runtime_dir: Option<OsString>) -> Option<PathBuf> {
    let non_empty = |value: &OsString| !value.is_empty();
    explicit.filter(non_empty).map(PathBuf::from).or_else(|| {
        runtime_dir
            .filter(non_empty)
            .map(|dir| PathBuf::from(dir).join(SOCKET_NAME))
    })
}

/// Sends `action` to the scoot this session runs.
pub fn send(action: ScootAction) -> Result<(), String> {
    let path = resolve(
        std::env::var_os(SOCKET_ENV),
        std::env::var_os("XDG_RUNTIME_DIR"),
    )
    .ok_or("cannot reach scoot: neither SCOOT_SOCKET nor XDG_RUNTIME_DIR is set")?;
    send_to(&path, action)
}

/// Sends `action` to the scoot listening at `path`.
fn send_to(path: &Path, action: ScootAction) -> Result<(), String> {
    let cannot =
        |e: &dyn std::fmt::Display| format!("cannot reach scoot at {}: {e}", path.display());
    let address = SocketAddrUnix::new(path).map_err(|e| cannot(&e))?;
    let fd = socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        None,
    )
    .map_err(|e| cannot(&e))?;
    connect(&fd, &address).map_err(|e| cannot(&e))?;
    let stream = UnixStream::from(fd);
    stream
        .set_nonblocking(false)
        .and_then(|()| stream.set_read_timeout(Some(TIMEOUT)))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|e| cannot(&e))?;
    let name = action.name();
    match (&stream).write_all(request(action).as_bytes()) {
        Ok(()) => {}
        // Scoot closed first: it is going away.
        Err(e) if gone(&e) => return Ok(()),
        Err(e) => return Err(format!("scoot did not take {name}: {e}")),
    }
    let mut reply = String::new();
    match BufReader::new((&stream).take(MAX_REPLY)).read_line(&mut reply) {
        // Closed with no reply: `quit` ended the session.
        Ok(0) => Ok(()),
        Ok(_) => match serde_json::from_str::<serde_json::Value>(&reply) {
            Ok(value) if value["type"] == "error" => Err(format!(
                "scoot refused {name}: {}",
                value["message"].as_str().unwrap_or("no reason given")
            )),
            _ => Ok(()),
        },
        Err(e) if gone(&e) => Ok(()),
        Err(e) => Err(format!("scoot did not answer {name}: {e}")),
    }
}

/// An error that means the other end closed the connection.
fn gone(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset | io::ErrorKind::BrokenPipe
    )
}
