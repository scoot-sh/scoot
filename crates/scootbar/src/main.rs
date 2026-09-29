//! `scootbar`: a status bar for Wayland.
//!
//! One binary. `scootbar daemon` connects to the compositor and gives every
//! output a bar, a `top`-layer surface along one edge that reserves its
//! space, showing modules (`modules`: the clock, so far). The plan and the
//! decisions behind it are in `docs/scootbar/`.
//!
//! No `unsafe` here: the two mappings it needs, the `wl_shm` buffer and a
//! font on a read-only mount, are `scootbg-mem`'s.

#![forbid(unsafe_code)]
// A build with no module (`--no-default-features`) keeps the module
// machinery with nothing to drive it; the default build checks it all.
#![cfg_attr(not(feature = "clock"), allow(dead_code))]

#[cfg(target_os = "linux")]
mod bar;
#[cfg(target_os = "linux")]
mod cli;
#[cfg(target_os = "linux")]
mod color;
#[cfg(target_os = "linux")]
mod config;
#[cfg(target_os = "linux")]
mod daemon;
#[cfg(target_os = "linux")]
mod density;
#[cfg(target_os = "linux")]
mod font;
#[cfg(target_os = "linux")]
mod layout;
#[cfg(target_os = "linux")]
mod modules;
#[cfg(target_os = "linux")]
mod outputs;
#[cfg(target_os = "linux")]
mod paint;
#[cfg(target_os = "linux")]
mod print;
#[cfg(target_os = "linux")]
mod render;
#[cfg(all(target_os = "linux", test))]
mod testfont;
#[cfg(target_os = "linux")]
mod text;
#[cfg(target_os = "linux")]
mod theme;

use std::process::ExitCode;

/// Exit status for a usage error, as for most Unix tools.
#[cfg(target_os = "linux")]
const USAGE_ERROR: u8 = 2;

#[cfg(target_os = "linux")]
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

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    use std::io::Write;
    // Not `eprintln!`, which panics if stderr is a closed pipe.
    let _ = writeln!(std::io::stderr(), "scootbar: runs on Linux only");
    ExitCode::FAILURE
}
