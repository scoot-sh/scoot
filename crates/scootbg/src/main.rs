//! `scootbg`: a wallpaper daemon for Wayland.
//!
//! One binary. `scootbg daemon` is the Wayland client and serves a control
//! socket; every other command sends it one request. The design, the
//! decided dependencies and the backlog live in `docs/scootbg/`.
//!
//! No `unsafe` here: all of it is in `scootbg-mem`.

#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
mod choices;
#[cfg(target_os = "linux")]
mod cli;
#[cfg(target_os = "linux")]
mod client;
#[cfg(target_os = "linux")]
mod color;
#[cfg(target_os = "linux")]
mod control;
#[cfg(target_os = "linux")]
mod daemon;
#[cfg(target_os = "linux")]
mod framing;
#[cfg(target_os = "linux")]
mod outputs;
#[cfg(target_os = "linux")]
mod paint;
#[cfg(target_os = "linux")]
mod paths;
#[cfg(target_os = "linux")]
mod print;
#[cfg(target_os = "linux")]
mod protocol;
#[cfg(target_os = "linux")]
mod waiters;

use std::process::ExitCode;

/// Blocks of 128 KiB and more are their own mappings, so a decode's heap
/// goes back to the kernel (dependencies-done.md §6b).
#[cfg(target_os = "linux")]
#[global_allocator]
static ALLOCATOR: scootbg_mem::LargeAlloc = scootbg_mem::LargeAlloc;

/// Exit status for a usage error, as for most Unix tools.
#[cfg(target_os = "linux")]
const USAGE_ERROR: u8 = 2;

#[cfg(target_os = "linux")]
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
        cli::Command::Help(topic) => print::print(topic.text()),
        cli::Command::Version => print::print(&format!("{}\n", cli::version_string())),
        cli::Command::Daemon => {
            return match daemon::run() {
                daemon::Exit::Stopped => ExitCode::SUCCESS,
                daemon::Exit::Failed(error) => {
                    warn(format_args!("scootbg: {error}"));
                    ExitCode::FAILURE
                }
            };
        }
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

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    use std::io::Write;
    // Not `eprintln!`, which panics if stderr is a closed pipe.
    let _ = writeln!(std::io::stderr(), "scootbg: runs on Linux only");
    ExitCode::FAILURE
}
