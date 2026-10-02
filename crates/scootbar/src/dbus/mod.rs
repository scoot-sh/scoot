//! The shared D-Bus client: the session (and later system) bus over one
//! multiplexed connection, on a poll-loop fd.
//!
//! The spike's verdict (`docs/scootbar/spikes/dbus-client.md`): a
//! hand-rolled minimal client wins on every measured row (332,456 B,
//! zero dependencies, 1 thread) and is the only option fitting the bar's
//! doctrine (one single-threaded `poll(2)` loop, no async runtime, no C
//! library). Its first consumer is the tray; notifications, MPRIS and
//! BlueZ hang off the same connection later (pending calls by serial,
//! consumer callbacks by match rule, `NameOwnerChanged` tracked
//! centrally).
//!
//! [`proto`] is the wire (framing, marshalling, shape readers; `std`
//! only, fuzzed); [`conn`] is the transport (auth, `Hello`, calls,
//! match rules, the pending-call table, owned per-turn events).

pub mod conn;
pub mod proto;

/// The shape checks, shared with the tray (pixmap and property walks)
/// and the fuzz target.
pub(crate) mod fuzz;

#[cfg(test)]
mod tests;
