//! flexwm: the compositor, and the client that drives it.

mod cli;
mod msg;
mod output;

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
            // on the EPIPE -- the same class of bug `output` fixes on
            // stdout. The process still exits FAILURE either way.
            output::warn(format_args!("flexwm: {error}"));
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    match cli::parse(std::env::args().skip(1))? {
        cli::Command::Help => {
            // `cli::USAGE` already ends in a newline, so `write_str`, not
            // `print_line`. A closed pipe is a quiet success here too:
            // `flexwm --help | head -1` exits 0.
            output::write_str(cli::USAGE)?;
            Ok(())
        }
        cli::Command::Msg { request, out } => msg::run(&request, out.as_deref()),
        cli::Command::Compositor(options) => start_compositor(options),
    }
}

#[cfg(target_os = "linux")]
fn start_compositor(options: cli::CompositorOptions) -> Result<(), Box<dyn Error>> {
    compositor::run(options)
}

#[cfg(not(target_os = "linux"))]
fn start_compositor(_options: cli::CompositorOptions) -> Result<(), Box<dyn Error>> {
    Err("the compositor only runs on Linux; `flexwm msg` works everywhere".into())
}
