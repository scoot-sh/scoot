//! The control socket: the server half (accepting, bounding and servicing
//! connections), the claim, line framing and socket paths live in
//! [`scootui::control`], shared with the other shell daemons; the
//! notifier's own request handling ([`protocol`], answered in
//! [`respond`](super::respond)), and the `scootnotify msg` client
//! ([`client`]) stay here.
//!
//! The re-exports below keep every `crate::control::…` path resolving, so
//! the daemon, the tests and the fakes name the moved items where they
//! always have.

pub mod client;
pub mod protocol;
pub mod respond;

pub use scootui::control::paths;
pub use scootui::control::{
    Claim, ClaimError, Handler, Kinds, MAX_CONNECTIONS, MAX_SUBSCRIBERS, Server,
};

#[cfg(test)]
mod tests;
