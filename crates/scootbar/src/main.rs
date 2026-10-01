//! `scootbar`: a status bar for Wayland.
//!
//! One binary. `scootbar daemon` connects to the compositor and gives every
//! output a bar, a `top`-layer surface along one edge that reserves its
//! space, showing modules (`modules`: the clock and workspaces, so far).
//! `scootbar msg` asks the running daemon over its control socket: `query`
//! reads each module's state as JSON (the agent hook), `reload` re-reads
//! the config file, and `version` and `kill` are what they sound like. The
//! plan and the decisions behind it are in `docs/scootbar/`.
//!
//! No `unsafe` here: the two mappings it needs, the `wl_shm` buffer and a
//! font file nothing can rewrite, are `scootbg-mem`'s.

#![forbid(unsafe_code)]
// A build with no module (`--no-default-features`) keeps the module
// machinery with nothing to drive it; the default build checks it all.
#![cfg_attr(not(feature = "clock"), allow(dead_code))]

mod action;
mod bar;
mod cli;
mod color;
mod config;
mod control;
mod daemon;
mod density;
mod font;
mod icon;
mod layout;
mod modules;
mod outputs;
mod paint;
mod pointer;
mod policy;
mod print;
mod region;
mod render;
mod scoot;
#[cfg(test)]
mod snapshots;
mod spawn;
#[cfg(test)]
mod testfds;
#[cfg(test)]
mod testfont;
mod text;
mod theme;

// Tests pinning the warm tick allocates nothing count heap allocations
// through this (scootbg_mem::count), forwarding everything else to the
// system allocator. Test-only: the shipped binary uses the default.
#[cfg(test)]
#[global_allocator]
static COUNTING_ALLOC: scootbg_mem::CountingAlloc = scootbg_mem::CountingAlloc;

use std::borrow::Cow;
use std::process::ExitCode;

/// Exit status for a usage error, as for most Unix tools.
const USAGE_ERROR: u8 = 2;

fn main() -> ExitCode {
    use print::warn;

    // A command of an `exec` module is started through this binary, which
    // arms its end with the bar's and becomes the command
    // (`modules::exec::guard`): it never reaches the CLI.
    #[cfg(feature = "exec")]
    if let Some(code) = modules::exec::guard::run(std::env::args_os().skip(1)) {
        return code;
    }
    let command = match cli::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            warn(format_args!("scootbar: {error}"));
            return ExitCode::from(USAGE_ERROR);
        }
    };
    match command {
        cli::Command::Help(topic) => print_out(topic.text()),
        cli::Command::Version => print_out(&format!("{}\n", cli::version_string())),
        cli::Command::Daemon(command) => run_daemon(*command),
        cli::Command::Msg(msg) => run_msg(msg),
    }
}

fn print_out(text: &str) -> ExitCode {
    use print::warn;
    match print::print(text) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            warn(format_args!("scootbar: {error}"));
            ExitCode::FAILURE
        }
    }
}

/// `daemon`: the config file, then the flags over it, then the bar. A bad
/// file is exit status 1 (the flags were fine); a flag the file clashes
/// with (a module placed twice between them) is a usage error.
fn run_daemon(command: cli::DaemonCommand) -> ExitCode {
    use print::warn;
    let startup = match config::load_startup(command.file.as_deref()) {
        Ok(startup) => startup,
        Err(error) => {
            warn(format_args!("scootbar: {error}"));
            return ExitCode::FAILURE;
        }
    };
    let config = if startup.from_file {
        let mut config = startup.config;
        if let Err(error) = command.given.overlay(&mut config) {
            warn(format_args!("scootbar: {error}"));
            return ExitCode::from(USAGE_ERROR);
        }
        config
    } else {
        *command.config
    };
    if command.check {
        return match daemon::check(&config) {
            Ok(()) => print_out("ok\n"),
            Err(error) => {
                warn(format_args!("scootbar: {error}"));
                ExitCode::FAILURE
            }
        };
    }
    match daemon::run(config, startup.file, command.given) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            warn(format_args!("scootbar: {error}"));
            ExitCode::FAILURE
        }
    }
}

/// `msg`: one request to the running daemon, one reply. `query`,
/// `version`, `reload`, `hide`, `show` and `toggle` print the reply; `kill` and `set` print nothing
/// on success, as scootbg's silent commands do.
fn run_msg(msg: cli::Msg) -> ExitCode {
    use print::warn;
    match msg {
        cli::Msg::Set { id, value } => {
            let value: serde_json::Value = match serde_json::from_str(&value) {
                // Unreachable: `cli` refused what is not JSON already. A
                // loud error, not a panic, if it ever happens.
                Err(error) => {
                    warn(format_args!("scootbar: the value is not JSON: {error}"));
                    return ExitCode::FAILURE;
                }
                Ok(value) => value,
            };
            send(
                &control::protocol::Request::Set {
                    id: Cow::Owned(id),
                    value,
                },
                false,
            )
        }
        cli::Msg::Query => send(&control::protocol::Request::Query, true),
        cli::Msg::Reload => send(&control::protocol::Request::Reload, true),
        cli::Msg::Hide => send(&control::protocol::Request::Hide, true),
        cli::Msg::Show => send(&control::protocol::Request::Show, true),
        cli::Msg::Toggle => send(&control::protocol::Request::Toggle, true),
        cli::Msg::Version => send(&control::protocol::Request::Version, true),
        cli::Msg::Kill => send(&control::protocol::Request::Kill, false),
    }
}

fn send(request: &control::protocol::Request<'_>, echoes: bool) -> ExitCode {
    use print::warn;
    match control::client::send(request) {
        Ok(_) if !echoes => ExitCode::SUCCESS,
        Ok(reply) => print_out(&reply),
        Err(error) => {
            warn(format_args!("scootbar: {error}"));
            ExitCode::FAILURE
        }
    }
}
