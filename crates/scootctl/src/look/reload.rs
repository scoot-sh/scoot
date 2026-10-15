//! The reload half of a swap: each app's control socket, scoped to the look.
//!
//! `scoot look apply` drives one scoped reload per running first-party app,
//! in order (the compositor, then every bar of the session), and aggregates
//! the replies. An app that is not running is skipped with a note, never a
//! refusal. A refusal rolls the swap back.
//!
//! The bar client here is deliberately the wire and nothing else (one JSON
//! line each way over the bar's socket): `scootbar` is a binary crate, so
//! its control module cannot be imported, and the compositor half goes
//! through `scoot-ipc`'s client. A compositor older than the look-scoped
//! tag answers it with an ordinary error, which is a refusal, never a
//! plain reload (a plain reload would apply the pending edits the scope
//! exists to skip). A bar older than the look scope answers a plain `ok`
//! without the `"scope":"look"` receipt: the CLI refuses it with one
//! clear line ("the running bar predates the look-scoped reload: restart
//! it"), never retrying as a plain reload.

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::FileTypeExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long a reload waits on an app that accepted the connection but does
/// not answer, before the swap refuses rather than hanging.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The longest bar reply read. Replies are one line (`ok` or `error`);
/// this only bounds a daemon gone wrong.
const MAX_REPLY: u64 = 64 * 1024;

/// What a scoped reload answered.
#[derive(Debug, Default, PartialEq)]
pub struct Reloaded {
    /// Fields the app re-applied live.
    pub applied: Vec<String>,
    /// Fields that differed but cannot apply live. Non-empty refuses.
    pub refused: Vec<String>,
    /// User-file keys that override a look token.
    pub shadowed: Vec<String>,
    /// User-file keys that failed to parse and fell back to the look.
    pub invalid: Vec<String>,
}

/// Why a scoped reload did not apply.
#[derive(Debug, PartialEq)]
pub enum ReloadError {
    /// Nothing listens: the app is not running. Skipped with a note.
    NotRunning(String),
    /// The app answered with a refusal (an older compositor's
    /// unknown-request error included: it is an ordinary error on the
    /// wire, and it refuses the swap rather than retrying as a plain
    /// reload).
    Refused(String),
}

/// One scoped reload against the compositor, so tests script the peer.
pub trait CompositorReload {
    fn reload_look(&mut self) -> Result<Reloaded, ReloadError>;
}

/// The live compositor over its IPC socket.
pub struct IpcCompositor {
    socket: Option<PathBuf>,
}

impl IpcCompositor {
    /// Reloads the session on the default socket (`$SCOOT_SOCKET` else
    /// `$XDG_RUNTIME_DIR/scoot.sock`).
    pub fn default_socket() -> Self {
        Self { socket: None }
    }

    /// Reloads the session on an explicit socket (`--socket`).
    pub fn explicit(socket: PathBuf) -> Self {
        Self {
            socket: Some(socket),
        }
    }
}

impl CompositorReload for IpcCompositor {
    fn reload_look(&mut self) -> Result<Reloaded, ReloadError> {
        let path = match &self.socket {
            Some(path) => path.clone(),
            None => scoot_ipc::socket_path().ok_or_else(|| {
                ReloadError::NotRunning(
                    "neither SCOOT_SOCKET nor XDG_RUNTIME_DIR is set".to_owned(),
                )
            })?,
        };
        let stream = std::os::unix::net::UnixStream::connect(&path).map_err(|error| {
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
            ) {
                ReloadError::NotRunning(format!("no compositor answers: {error}"))
            } else {
                ReloadError::Refused(format!("cannot reach the compositor: {error}"))
            }
        })?;
        // Bound a peer that accepts and never answers: a hung compositor
        // must refuse the swap, never wedge it with the lock held.
        stream
            .set_read_timeout(Some(TIMEOUT))
            .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
            .map_err(|error| {
                ReloadError::Refused(format!(
                    "cannot set a timeout on {}: {error}",
                    path.display()
                ))
            })?;
        let mut client = scoot_ipc::Client::from_stream(stream).map_err(|error| {
            ReloadError::Refused(format!("cannot reach the compositor: {error}"))
        })?;
        match client
            .request(&scoot_ipc::Request::ReloadLook)
            .map_err(|error| {
                let text = error.to_string();
                if text.contains("unknown variant") {
                    return ReloadError::Refused(
                        "the compositor predates scoot look: update scoot and restart the session"
                            .to_owned(),
                    );
                }
                ReloadError::Refused(format!("the compositor did not answer: {text}"))
            })? {
            scoot_ipc::Response::Reloaded {
                applied,
                refused,
                shadowed,
                invalid,
                scope,
            } => {
                // The receipt must be for the scoped reload that was asked:
                // a plain receipt answers a different request.
                if scope != Some(scoot_ipc::ReloadScope::Look) {
                    return Err(ReloadError::Refused(
                        "the compositor answered the look reload with a plain receipt: refusing it"
                            .to_owned(),
                    ));
                }
                if refused.is_empty() {
                    Ok(Reloaded {
                        applied,
                        refused,
                        shadowed,
                        invalid,
                    })
                } else {
                    Err(ReloadError::Refused(format!(
                        "the compositor refused the look reload: {}",
                        refused.join("; ")
                    )))
                }
            }
            scoot_ipc::Response::Error { message } => {
                // An older compositor answers the unknown `ReloadLook` tag
                // with a serde wall: map it to one line.
                if message.contains("unknown variant") && message.contains("reload") {
                    return Err(ReloadError::Refused(
                        "the compositor predates scoot look: update scoot and restart the session"
                            .to_owned(),
                    ));
                }
                Err(ReloadError::Refused(format!(
                    "the compositor refused the look reload: {message}"
                )))
            }
            other => Err(ReloadError::Refused(format!(
                "the compositor answered the look reload with an unexpected reply: {other:?}"
            ))),
        }
    }
}

/// One scoped reload against a bar, so tests script the peer.
pub trait BarReload {
    /// The display the bar serves, for notes.
    fn display(&self) -> &str;
    /// Sends the look-scoped reload: `Ok(true)` reloaded, `Ok(false)` the
    /// socket names nothing listening (skipped with a note, never a
    /// refusal), `Err` the daemon refused, which rolls the swap back.
    fn reload_look(&self) -> Result<bool, String>;
}

/// A running bar over its control socket.
pub struct SockBar {
    display: String,
    socket: PathBuf,
}

impl SockBar {
    pub fn new(display: String, socket: PathBuf) -> Self {
        Self { display, socket }
    }
}

impl BarReload for SockBar {
    fn display(&self) -> &str {
        &self.display
    }

    fn reload_look(&self) -> Result<bool, String> {
        bar_reload_outcome(&self.socket)
    }
}

/// One look-scoped reload against the bar at `socket`: `Ok(true)` the
/// daemon reloaded, `Ok(false)` nothing listens there (a stale socket from
/// a crashed bar, or a path that never was one: skipped with a note). A
/// refused reply, an unreadable reply and a peer that accepts and never
/// answers are `Err`: the daemon is there and said no.
pub fn bar_reload_outcome(socket: &Path) -> Result<bool, String> {
    let mut stream = match UnixStream::connect(socket) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(false);
        }
        Err(error) => {
            return Err(format!(
                "cannot reach the bar on {}: {error}",
                socket.display()
            ));
        }
    };

    stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|error| format!("cannot set a timeout on {}: {error}", socket.display()))?;
    stream
        .write_all(b"{\"protocol\":1,\"type\":\"reload\",\"scope\":\"look\"}\n")
        .map_err(|error| format!("cannot ask the bar on {}: {error}", socket.display()))?;
    let mut reader = BufReader::new((&stream).take(MAX_REPLY));
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|error| format!("the bar on {} did not answer: {error}", socket.display()))?;
    if line.is_empty() {
        return Err(format!(
            "the bar on {} closed the connection without replying",
            socket.display()
        ));
    }
    let reply: serde_json::Value = serde_json::from_str(&line).map_err(|error| {
        format!(
            "unreadable reply from the bar on {}: {error}",
            socket.display()
        )
    })?;
    match reply.get("type").and_then(|kind| kind.as_str()) {
        Some("ok") => {
            // The scoped reload must carry its scope receipt: a plain
            // `ok` answers a different request (an older bar that ignores
            // the unknown `scope` field and runs a plain reload, applying
            // pending edits). Refuse it loudly, never retry as plain.
            match reply.get("scope").and_then(|scope| scope.as_str()) {
                Some("look") => Ok(true),
                _ => Err("the running bar predates the look-scoped reload: restart it".to_owned()),
            }
        }
        Some("bar") => Ok(true),
        Some("error") => Err(format!(
            "the bar refused the look reload: {}",
            reply
                .get("message")
                .and_then(|message| message.as_str())
                .unwrap_or("(no message)")
        )),
        other => Err(format!(
            "the bar answered the look reload with an unexpected reply: {}",
            other.unwrap_or("(no type)")
        )),
    }
}

/// Sends one look-scoped reload to the bar at `socket`, for the unit tests:
/// an absent listener reads as a refusal here (production callers use
/// [`bar_reload_outcome`] directly and skip it with a note instead).
pub fn send_bar_reload(socket: &Path) -> Result<(), String> {
    match bar_reload_outcome(socket)? {
        true => Ok(()),
        false => Err(format!("nothing listens on {}", socket.display())),
    }
}

/// The runtime dir sockets live in: `$XDG_RUNTIME_DIR`, else
/// `/run/user/<uid>`. Empty counts as unset.
pub fn runtime_dir(xdg: Option<&OsStr>, uid: u32) -> Option<PathBuf> {
    if let Some(dir) = xdg.filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    Some(PathBuf::from(format!("/run/user/{uid}")))
}

/// Finds every bar of this session: the one on `$WAYLAND_DISPLAY` when it
/// names one, else every `scootbar-*.sock` in the runtime dir. A display
/// whose socket is absent is a note ("no bar on ...: skipped"), never a
/// refusal; so is an empty scan.
pub fn bar_targets() -> (Vec<SockBar>, Vec<String>) {
    let xdg = std::env::var_os("XDG_RUNTIME_DIR");
    let runtime = runtime_dir(xdg.as_deref(), current_uid());
    let display = std::env::var_os("WAYLAND_DISPLAY")
        .and_then(|display| display.to_str().map(str::to_owned))
        .filter(|display| !display.is_empty());
    match (display, runtime) {
        (Some(display), Some(runtime)) => {
            let name = display_name(&display);
            let socket = runtime.join(format!("scootbar-{name}.sock"));
            match is_live_socket(&socket) {
                true => (vec![SockBar::new(display, socket)], Vec::new()),
                false => (Vec::new(), vec![format!("no bar on {display}: skipped")]),
            }
        }
        (None, Some(runtime)) => {
            let mut bars: Vec<SockBar> = Vec::new();
            match std::fs::read_dir(&runtime) {
                Ok(entries) => {
                    let mut names: Vec<_> = entries
                        .filter_map(|entry| entry.ok())
                        .map(|entry| entry.file_name())
                        .collect();
                    names.sort();
                    for name in names {
                        let name = name.to_string_lossy().into_owned();
                        let Some(rest) = name.strip_prefix("scootbar-") else {
                            continue;
                        };
                        let Some(display) = rest.strip_suffix(".sock") else {
                            continue;
                        };
                        let socket = runtime.join(&name);
                        if is_live_socket(&socket) {
                            bars.push(SockBar::new(display.to_owned(), socket));
                        }
                    }
                }
                Err(error) => {
                    return (
                        Vec::new(),
                        vec![format!(
                            "cannot list {}: {error}: no bars reloaded",
                            runtime.display()
                        )],
                    );
                }
            }
            if bars.is_empty() {
                (
                    Vec::new(),
                    vec!["no running bars found: skipped".to_owned()],
                )
            } else {
                (bars, Vec::new())
            }
        }
        (_, None) => (
            Vec::new(),
            vec!["no runtime dir: no bars reloaded".to_owned()],
        ),
    }
}

/// The socket-name half of a display: the last `/`-component, sanitized to
/// `[A-Za-z0-9._-]` the way the bar names its socket (anything else becomes
/// `_`), so a display can neither escape the runtime dir nor miss the bar.
pub fn display_name(display: &str) -> String {
    let base = display.rsplit('/').next().unwrap_or(display);
    base.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Whether `socket` is a live socket file: `symlink_metadata` never
/// follows the path, so a dangling link is absent, and anything that is
/// not a socket (a stale regular file, a directory) is absent too. Only a
/// socket gets a reload attempt; anything else is a skip-with-note, never
/// a refusal.
fn is_live_socket(socket: &Path) -> bool {
    std::fs::symlink_metadata(socket)
        .map(|meta| meta.file_type().is_socket())
        .unwrap_or(false)
}

fn current_uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

#[cfg(test)]
mod tests;
