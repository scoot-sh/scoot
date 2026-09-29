//! `scootbar`: a status bar for Wayland.
//!
//! One binary. `scootbar daemon` connects to the compositor and gives every
//! output a bar, a `top`-layer surface along one edge that reserves its
//! space, showing modules (`modules`: the clock, so far). The plan and the
//! decisions behind it are in `docs/scootbar/`.
//!
//! No `unsafe` here: the two mappings it needs, the `wl_shm` buffer and a
//! font file nothing can rewrite, are `scootbg-mem`'s.

#![forbid(unsafe_code)]
// A build with no module (`--no-default-features`) keeps the module
// machinery with nothing to drive it; the default build checks it all.
#![cfg_attr(not(feature = "clock"), allow(dead_code))]

mod bar;
mod cli;
mod color;
mod config;
mod daemon;
mod density;
mod font;
mod layout;
mod modules;
mod outputs;
mod paint;
mod print;
mod render;
#[cfg(test)]
mod snapshots;
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

use std::process::ExitCode;

/// Exit status for a usage error, as for most Unix tools.
const USAGE_ERROR: u8 = 2;

fn main() -> ExitCode {
    use print::warn;

    let command = match cli::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            warn(format_args!("scootbar: {error}"));
            return ExitCode::from(USAGE_ERROR);
        }
    };
    let printed = match command {
        cli::Command::Help(topic) => print::print(topic.text()),
        cli::Command::Version => print::print(&format!("{}\n", cli::version_string())),
        cli::Command::Daemon(config) => {
            return match daemon::run(*config) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    warn(format_args!("scootbar: {error}"));
                    ExitCode::FAILURE
                }
            };
        }
    };
    match printed {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            warn(format_args!("scootbar: {error}"));
            ExitCode::FAILURE
        }
    }
}
