//! `scootbg`: a wallpaper daemon for Wayland.
//!
//! One binary. `scootbg daemon` is the Wayland client and serves a control
//! socket; every other command sends it one request. The design, the
//! decided dependencies and the backlog live in `docs/scootbg/`.
//!
//! No `unsafe` here: all of it is in `scootbg-mem`.

#![forbid(unsafe_code)]

mod apply;
mod choices;
mod cli;
mod client;
mod color;
mod control;
mod daemon;
mod density;
mod fetch;
mod framing;
mod help;
mod image;
mod jobs;
mod outputs;
mod paint;
mod paths;
mod print;
mod protocol;
mod rotation;
mod section;
mod sha256;
mod share;
mod state;
mod transition;
mod waiters;
mod wallpaper;

use std::process::ExitCode;

/// Blocks of 128 KiB and more are their own mappings, so a decode's heap
/// goes back to the kernel (dependencies-done.md §6b).
#[global_allocator]
static ALLOCATOR: scootbg_mem::LargeAlloc = scootbg_mem::LargeAlloc;

/// Exit status for a usage error, as for most Unix tools.
const USAGE_ERROR: u8 = 2;

fn main() -> ExitCode {
    use print::warn;

    let command = match cli::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            warn(format_args!("scootbg: {error}"));
            return ExitCode::from(USAGE_ERROR);
        }
    };
    let printed = match command {
        cli::Command::Help(topic) => print::print(&topic.text()),
        cli::Command::Version => print::print(&format!("{}\n", cli::version_string())),
        cli::Command::Daemon(options) => {
            return match daemon::run(options, None) {
                daemon::Exit::Stopped => ExitCode::SUCCESS,
                daemon::Exit::Failed(error) => {
                    warn(format_args!("scootbg: {error}"));
                    ExitCode::FAILURE
                }
            };
        }
        cli::Command::ApplyConfig(options) => return ExitCode::from(apply::run(options)),
        cli::Command::Client(request) => match client::send(&request) {
            // `kill`, `set` and `clear` print nothing on success; the
            // others print the reply.
            Ok(_)
                if matches!(
                    request,
                    protocol::Request::Kill
                        | protocol::Request::Set { .. }
                        | protocol::Request::Clear { .. }
                ) =>
            {
                Ok(())
            }
            Ok(reply) => print::print(&reply),
            Err(error) => {
                warn(format_args!("scootbg: {error}"));
                return ExitCode::FAILURE;
            }
        },
    };
    match printed {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            warn(format_args!("scootbg: {error}"));
            ExitCode::FAILURE
        }
    }
}
