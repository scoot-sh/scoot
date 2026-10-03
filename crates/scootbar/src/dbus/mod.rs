//! The shared D-Bus client: the session (and later system) bus over one
//! multiplexed connection, on a poll-loop fd.
//!
//! The spike's verdict (`docs/scootbar/spikes/dbus-client.md`): a
//! hand-rolled minimal client wins on every measured row (332,456 B,
//! zero dependencies, 1 thread) and is the only option fitting the bar's
//! doctrine (one single-threaded `poll(2)` loop, no async runtime, no C
//! library). Its first consumer is the tray and its second the media
//! module (MPRIS); notifications and BlueZ come later. Each consumer holds
//! its own connection (a second fd, no shared state between modules): one
//! shared connection was considered and decided against (the spike records
//! why: failure isolation, match rules that do not union, no measured
//! pressure), and each consumer matches the signals it wants.
//!
//! [`proto`] is the wire (framing, marshalling, shape readers; `std`
//! only, fuzzed); [`mpris`] is the media module's shape readers over it
//! (`std` only, fuzzed with it); [`conn`] is the transport (auth, `Hello`,
//! calls, match rules, the pending-call table, owned per-turn events);
//! [`link`] holds a connection across the bus coming and going (waiting on
//! the socket's directory, redialling, the quick-death latch).

// The tray and the media module each use part of the client; a build with
// one of them has the other's readers unused.
#![cfg_attr(not(all(feature = "tray", feature = "media")), allow(dead_code))]

pub mod conn;
#[cfg(any(feature = "tray", feature = "media"))]
pub mod link;
/// The media module's MPRIS shapes, over [`proto`]. Compiled in tests
/// whatever the features, as the fuzz check reads them.
#[cfg(any(feature = "media", test))]
pub mod mpris;
pub mod proto;

/// The fuzz target's check, compiled here only for the test that replays
/// its corpus (`crates/scootbar/fuzz` compiles the file itself). The
/// pixmap and property walks live in [`proto`], shared with the tray.
#[cfg(test)]
mod fuzz;

#[cfg(test)]
mod tests;

/// A real `dbus-daemon` for the tests that must meet one.
#[cfg(test)]
pub mod testdaemon;

#[cfg(test)]
mod daemon_tests;
