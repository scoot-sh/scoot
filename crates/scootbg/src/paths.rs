//! Where the control socket and its lock live.
//!
//! One daemon per Wayland display: `$XDG_RUNTIME_DIR/scootbg-NAME.sock`,
//! with `NAME` from `$WAYLAND_DISPLAY`, so a nested session and its host
//! never share one. `WAYLAND_DISPLAY` may be an absolute path (libwayland
//! allows it), so `NAME` is its final component, with any byte outside
//! `[A-Za-z0-9._-]` replaced by `_`. Two displays that differ only in such
//! bytes would share a socket; the second daemon then refuses loudly.
//!
//! Beside the socket sits `scootbg-NAME.lock`, which a daemon holds an
//! exclusive `flock` on for its whole life. The lock, not the socket, says
//! whether a daemon is alive: the kernel drops it when the process dies,
//! however it dies, so a stale socket is recognised by its free lock and
//! two daemons starting at once cannot both win. The lock file itself is
//! never removed (removing it would reopen that race), the way
//! libwayland leaves `wayland-N.lock`.

use std::ffi::OsStr;
use std::fmt;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

/// libwayland's own default when `WAYLAND_DISPLAY` is unset.
pub const DEFAULT_DISPLAY: &str = "wayland-0";

/// `sun_path` holds 108 bytes on Linux, one of them the terminating NUL.
const MAX_SOCKET_PATH: usize = 107;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// The display name the paths were derived from, for messages.
    pub display: String,
    pub socket: PathBuf,
    pub lock: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    NoRuntimeDir,
    RelativeRuntimeDir(PathBuf),
    /// `WAYLAND_DISPLAY` is set but names nothing (empty after its last
    /// `/`).
    NoDisplayName(String),
    TooLong(PathBuf),
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRuntimeDir => write!(f, "XDG_RUNTIME_DIR is not set"),
            Self::RelativeRuntimeDir(dir) => {
                write!(f, "XDG_RUNTIME_DIR is not absolute: {}", dir.display())
            }
            Self::NoDisplayName(value) => {
                write!(
                    f,
                    "WAYLAND_DISPLAY `{value}` has no name to derive a socket from"
                )
            }
            Self::TooLong(path) => write!(
                f,
                "the control socket path is longer than a Unix socket allows \
                 ({MAX_SOCKET_PATH} bytes): {}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for PathError {}

/// The paths for this process's environment.
pub fn from_env() -> Result<Paths, PathError> {
    resolve(
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
        std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
    )
}

/// The paths for a given `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR`; unset
/// and empty mean the same, as in libwayland.
pub fn resolve(
    wayland_display: Option<&OsStr>,
    runtime_dir: Option<&OsStr>,
) -> Result<Paths, PathError> {
    let dir = runtime_dir
        .filter(|d| !d.is_empty())
        .map(Path::new)
        .ok_or(PathError::NoRuntimeDir)?;
    if !dir.is_absolute() {
        return Err(PathError::RelativeRuntimeDir(dir.to_path_buf()));
    }
    let display = display_name(wayland_display)?;
    let socket = dir.join(format!("scootbg-{display}.sock"));
    let lock = dir.join(format!("scootbg-{display}.lock"));
    // The lock path is the same length, so one check covers both.
    if socket.as_os_str().len() > MAX_SOCKET_PATH {
        return Err(PathError::TooLong(socket));
    }
    Ok(Paths {
        display,
        socket,
        lock,
    })
}

/// The sanitised final component of `WAYLAND_DISPLAY`.
pub fn display_name(wayland_display: Option<&OsStr>) -> Result<String, PathError> {
    let Some(value) = wayland_display.filter(|v| !v.is_empty()) else {
        return Ok(DEFAULT_DISPLAY.to_owned());
    };
    let bytes = value.as_bytes();
    let last = match bytes.iter().rposition(|&b| b == b'/') {
        Some(slash) => &bytes[slash + 1..],
        None => bytes,
    };
    if last.is_empty() {
        return Err(PathError::NoDisplayName(
            value.to_string_lossy().into_owned(),
        ));
    }
    Ok(last
        .iter()
        .map(|&b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-') {
                char::from(b)
            } else {
                '_'
            }
        })
        .collect())
}
