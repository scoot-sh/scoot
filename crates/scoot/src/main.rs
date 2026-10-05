//! scoot: the compositor, with the `msg` client alias.
//!
//! Starting the session is this binary's own job; talking to a running one
//! is `scootctl`'s. `scoot msg ...` stays as a permanent alias for it --
//! parsed and run through that crate, never reimplemented here, so the two
//! entry points cannot drift.

mod cli;

#[cfg(target_os = "linux")]
mod compositor;

use std::process::ExitCode;

/// Exit status for a usage error, as for most Unix tools (and as `scootbar`
/// and `scootbg` already do): the invocation was wrong, so the error names
/// what was wrong on stderr. Anything else that fails is exit status 1.
const USAGE_ERROR: u8 = 2;

/// How the process failed: a wrong invocation (exit 2, with a guess where
/// one applies) or a failed run (exit 1).
enum Failure {
    Usage(scootctl::Error),
    Runtime(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(error) => write!(f, "{error}"),
            Self::Runtime(message) => write!(f, "{message}"),
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Infallible by design: if stderr is closed too, there is
            // nowhere to report to, and `eprintln!` would panic (exit 101)
            // on the EPIPE -- the same class of bug `scootctl::output` fixes
            // on stdout. The process still exits FAILURE either way.
            scootctl::output::warn(format_args!("scoot: {error}"));
            ExitCode::from(match error {
                Failure::Usage(_) => USAGE_ERROR,
                Failure::Runtime(_) => 1,
            })
        }
    }
}

fn run() -> Result<(), Failure> {
    // A usage error (a wrong flag, verb or value) never starts anything: it
    // is `Failure::Usage` (exit 2) here, while everything past parsing is
    // `Failure::Runtime` (exit 1).
    match cli::parse(std::env::args().skip(1)).map_err(Failure::Usage)? {
        cli::Command::Help(page) => {
            // Help text already ends in a newline, so `write_str`, not
            // `print_line` -- and the same EPIPE contract as before: a closed
            // pipe (`scoot --help | head -c0`) is a quiet success, not a
            // panic, because the truncated consumer got what it wanted.
            scootctl::output::write_str(&page.text())
                .map_err(|error| Failure::Runtime(error.to_string()))?;
            Ok(())
        }
        cli::Command::Version => {
            // One `\n`-terminated line, so `print_line`, not `write_str`.
            // A closed pipe is a quiet success here too:
            // `scoot --version | head -c0` exits 0. Needs no compositor --
            // identifying a build without starting it is the whole point --
            // so this arm runs on every platform, like `--help`.
            scootctl::output::print_line(&scootctl::version_string())
                .map_err(|error| Failure::Runtime(error.to_string()))?;
            Ok(())
        }
        cli::Command::Msg { request, out } => scootctl::run(&request, out.as_deref())
            .map_err(|error| Failure::Runtime(error.to_string())),
        cli::Command::PrintDefaultConfig { write } => {
            print_default_config(write).map_err(|error| Failure::Runtime(error.to_string()))
        }
        cli::Command::Compositor(options) => {
            start_compositor(options).map_err(|error| Failure::Runtime(error.to_string()))
        }
    }
}

#[cfg(target_os = "linux")]
fn print_default_config(write: bool) -> Result<(), Box<dyn std::error::Error>> {
    if write {
        // The refuse-to-overwrite convenience: same bytes stdout would have
        // carried, placed at the default config location instead. The one
        // summary line goes through the same EPIPE contract (`scoot
        // --print-default-config --write | head -c0` is a quiet success);
        // a refusal is an `Err`, which `main` reports on stderr and exits
        // FAILURE for -- loud even when stdout is closed.
        let path = compositor::config::write_default_config()?;
        scootctl::output::print_line(&format!("wrote {}", path.display()))?;
        return Ok(());
    }
    // Help text already ends in a newline, so `write_str`, not `print_line`
    // -- and the same EPIPE contract as `--help`: a closed pipe
    // (`scoot --print-default-config | head -c0`) is a quiet success, not a
    // panic, because the truncated consumer got what it wanted.
    scootctl::output::write_str(compositor::config::default_config_toml().as_str())?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn print_default_config(_write: bool) -> Result<(), Box<dyn std::error::Error>> {
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
fn start_compositor(options: cli::CompositorOptions) -> Result<(), Box<dyn std::error::Error>> {
    compositor::run(options)
}

#[cfg(not(target_os = "linux"))]
fn start_compositor(_options: cli::CompositorOptions) -> Result<(), Box<dyn std::error::Error>> {
    Err("the compositor only runs on Linux; `scoot msg` works everywhere".into())
}
