//! The scoot control protocol: newline-delimited JSON over a Unix socket.
//!
//! A connection is a simple loop -- write one [`Request`] per line, read one
//! [`Response`] per line. Nothing here is Wayland- or macOS-specific, so any
//! scoot shell can host it and any script or agent can speak it.

#![forbid(unsafe_code)]

mod action;
#[cfg(unix)]
mod client;
mod codec;
#[cfg(feature = "core")]
mod convert;
mod key;
mod request;
mod response;
mod socket;

pub use action::{Action, Horizontal, Vertical};
#[cfg(unix)]
pub use client::Client;
pub use codec::{decode, encode, read_message, read_message_buffered, write_message};
pub use key::{KeyCombo, Modifier, ParseKeyComboError};
pub use request::{PointerButton, Request};
pub use response::{OutputSnapshot, Rect, Response, Screenshot, WindowSnapshot};
pub use socket::{SOCKET_ENV, socket_path};

/// Bumped whenever a wire-format change would break existing clients.
///
/// `Response` is internally tagged (`#[serde(tag = "type", ...)]`), and this
/// scheme's own `unknown_request_types_are_rejected` test (see
/// `scoot-ipc/tests/wire.rs`) proves an unrecognized tag is a hard decode
/// error, not something an older client can silently ignore -- so adding
/// `Response::Warning` (2026-09, the IPC-VT-switch-warning item) is exactly
/// the kind of change this constant's doc warns about: a client built
/// against protocol 1 fails to decode a reply it's never seen the moment
/// the server sends one, rather than degrading gracefully.
pub const PROTOCOL_VERSION: u32 = 2;
