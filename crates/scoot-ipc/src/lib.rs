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
mod event;
mod key;
mod request;
mod response;
mod socket;

pub use action::{Action, Horizontal, Vertical};
#[cfg(unix)]
pub use client::Client;
pub use codec::{decode, encode, read_message, read_message_buffered, write_message};
pub use event::{
    EventKind, KeyboardLayout, OutputChanged, OutputRemoved, OutputRestored, WorkspaceSnapshot,
};
pub use key::{KeyCombo, Modifier, ParseKeyComboError};
pub use request::{PointerButton, Request, SCREENSHOT_CURSOR_DEFAULT};
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
///
/// The same bar moved this 2 → 3 for `Response::Reloaded` (2026-09, config
/// reload): only a client new enough to send `Request::Reload` ever receives
/// one, but the variant is still a new tag on the wire, and an older client
/// handed one would fail its decode. What did *not* move it is the request
/// half -- an unknown `Request` tag is a decode error the server answers
/// with an ordinary `Error` and keeps serving (see `unknown_request_types_are_rejected`),
/// so an older server meets a new `reload` client with an error, not a kill.
///
/// And the same bar moves it 3 → 4 for the event subscription (2026-09, the
/// output removed/restored event): `Response::Subscribed`,
/// `Response::OutputRemoved` and `Response::OutputRestored` are three new
/// tags at once, under one bump. A client that never sends
/// `Request::Subscribe` never receives any of them -- the bump costs
/// existing clients nothing at runtime -- while an unknown event kind in a
/// `subscribe` is answered with an ordinary `Error` like any unknown
/// request tag, so an older server meets a newer subscriber the same way.
///
/// And again 4 → 5 for the output-changed event (2026-10): one new tag,
/// `Response::OutputChanged`, under the same rule -- only a subscriber ever
/// receives one.
///
/// And again 5 → 6 for the keyboard layout query and event (2026-10): two
/// new tags, `Response::Keyboard` and `Response::KeyboardChanged`, under the
/// same rule -- only a client new enough to ask or subscribe ever receives
/// one.
///
/// And again 6 → 7 for the workspace occupancy event (2026-10): one new
/// tag, `Response::Workspaces`, under the same rule -- only a subscriber
/// ever receives one.
///
/// And again 7 → 8 for the side-effect-free lock query and event (2026-10):
/// two new tags, `Response::Locked` and `Response::LockChanged`, under the
/// same rule -- only a client new enough to ask or subscribe ever receives
/// one. An older server meets the new `locked` request tag and the new
/// `lock` subscribe kind with an ordinary `Error`, like any unknown tag.
pub const PROTOCOL_VERSION: u32 = 8;
