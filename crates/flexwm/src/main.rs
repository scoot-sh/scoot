//! flexwm: the compositor, and the client that drives it.

mod cli;
mod msg;

#[cfg(target_os = "linux")]
mod compositor;

use std::error::Error;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("flexwm: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    match cli::parse(std::env::args().skip(1))? {
        cli::Command::Help => {
            print!("{}", cli::USAGE);
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
