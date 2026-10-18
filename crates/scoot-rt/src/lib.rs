//! `scoot-rt`: the leaf both `scootui` and `scoot-dbus` depend on.
//!
//! Panic-free printing and the runtime directory, with `std` and `rustix`
//! only. Extracted from `scootbar` (`src/print.rs`, `src/control/paths.rs`'s
//! `runtime_dir`) so the drawing crate and the D-Bus crate share one copy
//! instead of each carrying its own.
//!
//! No `unsafe` here.

#![forbid(unsafe_code)]

use std::path::PathBuf;

pub mod print;

/// The runtime directory: `$XDG_RUNTIME_DIR`, or its conventional
/// fallback. Unlike the control socket's directory, unset here falls back
/// instead of erroring: the session bus, the pulse socket and a
/// directory watch all wait on the fallback rather than refuse. One
/// helper for all three (the D-Bus client, the tray and the volume
/// module), so the fallback cannot drift between them.
pub fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    PathBuf::from(format!("/run/user/{}", rustix::process::getuid().as_raw()))
}
