//! scoot: the compositor, with the `msg` client alias.
//!
//! Starting the session is this binary's own job; talking to a running one
//! is `scootctl`'s. `scoot msg ...` stays as a permanent alias for it --
//! parsed and run through that crate, never reimplemented here, so the two
//! entry points cannot drift.

mod cli;

#[cfg(target_os = "linux")]
mod compositor;

use std::error::Error;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Infallible by design: if stderr is closed too, there is
            // nowhere to report to, and `eprintln!` would panic (exit 101)
            // on the EPIPE -- the same class of bug `scootctl::output` fixes
            // on stdout. The process still exits FAILURE either way.
            scootctl::output::warn(format_args!("scoot: {error}"));
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    match cli::parse(std::env::args().skip(1))? {
        cli::Command::Help => {
            // `cli::USAGE` already ends in a newline, so `write_str`, not
            // `print_line`. A closed pipe is a quiet success here too:
            // `scoot --help | head -1` exits 0.
            scootctl::output::write_str(cli::USAGE)?;
            Ok(())
        }
        cli::Command::Version => {
            // One `\n`-terminated line, so `print_line`, not `write_str`.
            // A closed pipe is a quiet success here too:
            // `scoot --version | head -c0` exits 0. Needs no compositor --
            // identifying a build without starting it is the whole point --
            // so this arm runs on every platform, like `--help`.
            scootctl::output::print_line(&scootctl::version_string())?;
            Ok(())
        }
        cli::Command::Msg { request, out } => scootctl::run(&request, out.as_deref()),
        cli::Command::PrintDefaultConfig => print_default_config(),
        cli::Command::Compositor(options) => start_compositor(options),
    }
}

#[cfg(target_os = "linux")]
fn print_default_config() -> Result<(), Box<dyn Error>> {
    // `USAGE` already ends in a newline, so `write_str`, not `print_line` --
    // and the same EPIPE contract as `--help`: a closed pipe
    // (`scoot --print-default-config | head -c0`) is a quiet success, not a
    // panic, because the truncated consumer got what it wanted.
    scootctl::output::write_str(compositor::config::default_config_toml().as_str())?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn print_default_config() -> Result<(), Box<dyn Error>> {
    // The emission is generated from the compositor's own defaults, which
    // live in the Linux-gated config module -- so there is no `scoot` binary
    // here that could emit them. Copy the example out of
    // `docs/configuration.md` instead.
    Err(
        "`--print-default-config` needs the compositor's defaults, which only exist on Linux"
            .into(),
    )
}

#[cfg(target_os = "linux")]
fn start_compositor(options: cli::CompositorOptions) -> Result<(), Box<dyn Error>> {
    compositor::run(options)
}

#[cfg(not(target_os = "linux"))]
fn start_compositor(_options: cli::CompositorOptions) -> Result<(), Box<dyn Error>> {
    Err("the compositor only runs on Linux; `scoot msg` works everywhere".into())
}
