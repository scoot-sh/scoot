//! `scootctl`: the remote-control client for the scoot compositor.
//!
//! One request per invocation, over the IPC socket -- the client half of the
//! computer-use story. All parsing and running lives in the library; this
//! front-end is argv in, exit code out. (`scoot msg ...` is the same client
//! kept as an alias on the compositor binary -- see `scootctl`'s crate doc
//! for why the two cannot drift.)

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
            scootctl::output::warn(format_args!("scootctl: {error}"));
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    match scootctl::parse(std::env::args().skip(1))? {
        scootctl::Command::Help => {
            // `cli::USAGE` already ends in a newline, so `write_str`, not
            // `print_line`. A closed pipe is a quiet success here too:
            // `scootctl --help | head -1` exits 0.
            scootctl::output::write_str(scootctl::USAGE)?;
            Ok(())
        }
        scootctl::Command::Msg { request, out } => scootctl::run(&request, out.as_deref()),
    }
}
