//! `scootctl`: the remote-control client for the scoot compositor.
//!
//! This crate is a *client* of the compositor and nothing else: it parses one
//! request from argv, sends it over the IPC socket, and prints the reply. It
//! never starts a compositor, never touches Wayland, and builds on every
//! platform (`scoot-ipc`'s client half plus `serde_json` plus std) -- which is
//! why the macOS package is this crate, not the compositor.
//!
//! The `scoot` binary keeps `scoot msg ...` as a permanent alias, but it owns
//! none of the client surface: its `Command::Msg` arm parses and runs through
//! this crate, so the two entry points cannot drift. The request grammar, the
//! `Error` display strings, and the help text below have exactly one owner --
//! here -- with containment tests on both sides pinning that `scoot --help`
//! prints the same `REQUESTS_HELP`/`ACTIONS_HELP` blocks `scootctl --help`
//! does.

pub mod cli;
pub mod msg;
pub mod output;

pub use cli::{Command, Error, Msg, USAGE, action, parse, parse_msg, version_string};
pub use msg::run;
