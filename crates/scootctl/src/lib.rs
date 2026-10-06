//! The client library behind `scoot msg`: the remote-control client for the
//! scoot compositor.
//!
//! This crate is a *client* of the compositor and nothing else: it parses one
//! request from argv, sends it over the IPC socket, and prints the reply. It
//! never starts a compositor, never touches Wayland, and builds on every
//! platform (`scoot-ipc`'s client half plus `serde_json` plus std) -- which is
//! why `scoot msg` drives a remote session from a Mac.
//!
//! The `scoot` binary owns none of the client surface: its `Command::Msg` arm
//! parses and runs through this crate, so there is exactly one grammar, one
//! set of `Error` display strings, and one help table -- here -- with
//! containment tests on both sides pinning that `scoot --help` prints the
//! same `REQUESTS_HELP`/`ACTIONS_HELP` blocks `scoot msg --help` does.
//!
//! There is no standalone client binary: `scoot msg` is the only client name.
//! (The crate keeps its historical name so every `use scootctl::` across the
//! workspace keeps working; renaming it would churn imports for no
//! user-visible gain.)

pub mod binds;
pub mod cli;
pub mod help;
pub mod msg;
pub mod output;

pub use cli::{Error, Msg, action, parse_msg, version_string};
pub use msg::run;
