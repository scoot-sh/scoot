//! What a panic does to the daemon.
//!
//! The release profile aborts on panic, so without this a panic would end
//! the daemon with SIGABRT and leave its socket file. That file is
//! harmless (a free lock marks it stale, see `control::claim`), but one
//! panic in particular is not a bug of ours and should not look like a
//! crash: `eprintln!` and `println!` panic when their stream is a pipe
//! whose reader has gone. `wayland-backend` reports connection errors with
//! `eprintln!` unless its `log` feature is on, and that feature compiles C
//! (`wayland-backend/build.rs` builds `log_shim.c` with `cc`), which
//! scootbg does not allow. So the hook:
//!
//! - removes the socket file on any panic while the daemon still owns it
//!   (see [`Armed`]), so no client connects to a daemon that is going away;
//! - for a "failed printing to stderr/stdout" panic, exits with status 1,
//!   the status of any other lost connection, instead of aborting;
//! - otherwise hands over to the default hook (which prints the panic if
//!   it can, never panicking itself) and the abort that follows, so a real
//!   bug still crashes loudly.
//!
//! scootbg's own writes never panic (`print`); only dependencies' can.

use std::panic::{self, PanicHookInfo};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The hook's hold on the socket file: while armed, a panic removes it.
/// Disarm before the claim is released: once the lock is free a new
/// daemon may bind the same path, and a panic later in this one's
/// shutdown (closing clients, dropping the Wayland connection) must not
/// remove the new daemon's socket.
#[derive(Debug, Clone)]
pub struct Armed(Arc<AtomicBool>);

impl Armed {
    #[cfg(test)]
    pub fn for_test(flag: Arc<AtomicBool>) -> Self {
        Self(flag)
    }

    pub fn disarm(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Installs the hook for a daemon that owns `socket`, armed.
pub fn install(socket: PathBuf) -> Armed {
    let armed = Armed(Arc::new(AtomicBool::new(true)));
    let hook_armed = armed.clone();
    let default = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        remove_if_armed(&hook_armed, &socket);
        if is_broken_stdio(info) {
            std::process::exit(1);
        }
        default(info);
    }));
    armed
}

/// Removes `socket` if the daemon still owns it, at most once.
pub fn remove_if_armed(armed: &Armed, socket: &Path) {
    if armed.0.swap(false, Ordering::SeqCst) {
        let _ = std::fs::remove_file(socket);
    }
}

/// Whether this is std's panic for a failed `print!`/`eprint!`
/// (`library/std/src/io/stdio.rs`: "failed printing to {label}: {e}").
/// Matched by message because std offers nothing more precise; if the
/// wording ever changes, the default hook and abort apply as before.
pub fn is_broken_stdio(info: &PanicHookInfo<'_>) -> bool {
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str));
    message.is_some_and(is_stdio_message)
}

pub fn is_stdio_message(message: &str) -> bool {
    message.starts_with("failed printing to stderr")
        || message.starts_with("failed printing to stdout")
}
