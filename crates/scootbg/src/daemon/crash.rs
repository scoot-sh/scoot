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
//! - removes the socket file on any panic, so no client connects to a
//!   daemon that is going away;
//! - for a "failed printing to stderr/stdout" panic, exits with status 1,
//!   the status of any other lost connection, instead of aborting;
//! - otherwise hands over to the default hook (which prints the panic if
//!   it can, never panicking itself) and the abort that follows, so a real
//!   bug still crashes loudly.
//!
//! scootbg's own writes never panic (`output`); only dependencies' can.

use std::panic::{self, PanicHookInfo};
use std::path::PathBuf;

/// Installs the hook for a daemon that owns `socket`.
pub fn install(socket: PathBuf) {
    let default = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = std::fs::remove_file(&socket);
        if is_broken_stdio(info) {
            std::process::exit(1);
        }
        default(info);
    }));
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
