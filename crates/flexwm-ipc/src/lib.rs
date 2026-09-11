//! The flexwm control protocol: newline-delimited JSON over a Unix socket.
//!
//! A connection is a simple loop -- write one [`Request`] per line, read one
//! [`Response`] per line. Nothing here is Wayland- or macOS-specific, so any
//! flexwm shell can host it and any script or agent can speak it.

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
pub use codec::{decode, encode, read_message, write_message};
pub use key::{KeyCombo, Modifier, ParseKeyComboError};
pub use request::{PointerButton, Request};
pub use response::{OutputSnapshot, Rect, Response, Screenshot, WindowSnapshot};
pub use socket::{SOCKET_ENV, socket_path};

/// Bumped whenever a wire-format change would break existing clients.
pub const PROTOCOL_VERSION: u32 = 1;
