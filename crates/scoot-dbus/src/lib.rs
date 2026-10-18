//! The shared D-Bus client: the session and system buses over one
//! multiplexed connection each, on a poll-loop fd.
//!
//! The spike's verdict (`dev/spikes/scootbar/dbus-client.md`): a
//! hand-rolled minimal client wins on every measured row (332,456 B,
//! zero dependencies, 1 thread) and is the only option fitting the bar's
//! doctrine (one single-threaded `poll(2)` loop, no async runtime, no C
//! library). Its first consumer is the tray and its second the media
//! module (MPRIS), both on the session bus; its third is the bluetooth
//! module (BlueZ) on the system bus. Each consumer holds
//! its own connection so far (a second fd, no shared state between
//! modules): the one shared connection the spike imagined (pending calls by
//! serial, consumer callbacks by match rule, `NameOwnerChanged` tracked
//! centrally) is not built, and each consumer matches the signals it wants.
//!
//! [`proto`] is the wire (framing, marshalling, shape readers; `std`
//! only, fuzzed); [`mpris`] is the media module's shape readers over it
//! (`std` only, fuzzed with it); [`bluez`] is the bluetooth module's BlueZ
//! shapes over it (the same); [`conn`] is the transport (auth, `Hello`,
//! calls, match rules, the pending-call table, owned per-turn events);
//! [`link`] holds a connection across the bus coming and going (waiting on
//! the socket's directory, redialling, the quick-death latch).
//!
//! Extracted from `scootbar` (`src/dbus/`) so `scootnotify` (which serves
//! `org.freedesktop.Notifications`), `scootlaunch` (which calls
//! `systemd1`) and `scootgreet` (which calls `login1`) share the one
//! client. What it needs that it lacks (an error body, a reply to an
//! over-cap call, a per-connection cap) lands with its consumer, not
//! here (`docs/scootbar/backlog/scoot-dbus-extract.md`, Not in this
//! ticket).
//!
//! No `unsafe` here.

#![forbid(unsafe_code)]
// The client's refusal idiom is `Err(())`: an address that names no
// dialable transport, a misshapen body, a refused frame. It carries no
// information by design (a refusal names its reason on stderr where a
// reason exists), so a custom error type would add variants without
// readers. Re-typing every reader would be a redesign, not this move.
#![allow(clippy::result_unit_err)]

// Linux only, like scootbar: the inotify watch and the timerfd are
// Linux-only, and the shell apps never run on a Mac.
#[cfg(not(target_os = "linux"))]
compile_error!(
    "scoot-dbus runs on Linux only, like scootbar; on a Mac, \
     cargo check --workspace --exclude scootbar --exclude scootbg --exclude scootbg-mem --exclude scootui --exclude scoot-dbus \
     (dev/README.md)"
);

use std::path::Path;
use std::sync::OnceLock;

// Tests pinning allocations count heap allocations through this
// (scootbg_mem::count), forwarding everything else to the system
// allocator. Test-only: the shipped crate links the default.
#[cfg(test)]
#[global_allocator]
static COUNTING_ALLOC: scootbg_mem::CountingAlloc = scootbg_mem::CountingAlloc;

pub mod bluez;
pub mod conn;
pub mod link;
pub mod mpris;
pub mod proto;

/// A real `dbus-daemon`, for the tests that must meet one. Test support
/// for this crate's own daemon tests and for the apps' integration tests
/// (which speak to a private bus), like `scootui`'s `testfont`: compiled
/// in always so the apps' `#[cfg(test)]` code can reach it, used only by
/// tests.
pub mod testdaemon;

/// The fuzz target's check, compiled here only for the test that replays
/// its corpus (`crates/scoot-dbus/fuzz` compiles the file itself). The
/// pixmap and property walks live in [`proto`], shared with the tray.
#[cfg(test)]
mod fuzz;

#[cfg(test)]
mod tests;

/// The client against a real `dbus-daemon`.
#[cfg(test)]
mod daemon_tests;

/// The running binary's name, for the lines said on stderr: read once
/// from `argv[0]`'s file name, so the same client warns as `scootbar:`,
/// `scootnotify:` or whichever binary linked it. Falls back to this
/// crate's name when there is no `argv[0]` to read, which is never on a
/// Unix machine but keeps the prefix total.
pub(crate) fn prog() -> &'static str {
    static PROG: OnceLock<String> = OnceLock::new();
    PROG.get_or_init(|| {
        std::env::args_os()
            .next()
            .map(|arg| {
                Path::new(&arg)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| String::from("scoot-dbus"))
    })
}
