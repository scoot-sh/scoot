//! `scootctl`: the remote-control client for the scoot compositor.
//!
//! One request per invocation, over the IPC socket -- the client half of the
//! computer-use story. All parsing and running lives in the library; this
//! front-end is argv in, exit code out. (`scoot msg ...` is the same client
//! kept as an alias on the compositor binary -- see `scootctl`'s crate doc
//! for why the two cannot drift.)

use std::process::ExitCode;

/// Exit status for a usage error, as for most Unix tools (and as `scootbar`
/// and `scootbg` already do): the invocation was wrong, so the error names
/// what was wrong on stderr. Anything else that fails -- no daemon, a
/// refused request, a failed write -- is exit status 1.
const USAGE_ERROR: u8 = 2;

/// How the process failed: a wrong invocation (exit 2, with a guess where
/// one applies) or a failed request (exit 1).
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
            // on the EPIPE -- the same class of bug `output` fixes on
            // stdout. The process still exits FAILURE either way.
            scootctl::output::warn(format_args!("scootctl: {error}"));
            ExitCode::from(match error {
                Failure::Usage(_) => USAGE_ERROR,
                Failure::Runtime(_) => 1,
            })
        }
    }
}

fn run() -> Result<(), Failure> {
    // A usage error (a wrong verb, flag or value) never reaches the socket:
    // it is `Failure::Usage` (exit 2) here, while everything past parsing
    // is `Failure::Runtime` (exit 1).
    match scootctl::parse(std::env::args().skip(1)).map_err(Failure::Usage)? {
        // Help text already ends in a newline, so `write_str`, not
        // `print_line`. A closed pipe is a quiet success here too:
        // `scootctl --help | head -1` exits 0.
        scootctl::Command::Help => {
            scootctl::output::write_str(&scootctl::usage())
                .map_err(|error| Failure::Runtime(error.to_string()))?;
            Ok(())
        }
        scootctl::Command::Topic(topic) => {
            scootctl::output::write_str(&scootctl::help::topic_text(topic))
                .map_err(|error| Failure::Runtime(error.to_string()))?;
            Ok(())
        }
        scootctl::Command::Verb { verb } => {
            let row = scootctl::help::verb_text(&verb).unwrap_or_else(|| scootctl::usage());
            scootctl::output::write_str(&row)
                .map_err(|error| Failure::Runtime(error.to_string()))?;
            Ok(())
        }
        scootctl::Command::Json => {
            // A single `\n`-terminated document, so `print_line`.
            scootctl::output::print_line(&scootctl::help::json("scootctl"))
                .map_err(|error| Failure::Runtime(error.to_string()))?;
            Ok(())
        }
        scootctl::Command::Version => {
            // A single `\n`-terminated line, so `print_line`, not
            // `write_str`. A closed pipe is a quiet success here too:
            // `scootctl --version | head -c0` exits 0.
            scootctl::output::print_line(&scootctl::version_string())
                .map_err(|error| Failure::Runtime(error.to_string()))?;
            Ok(())
        }
        scootctl::Command::Msg { request, out } => scootctl::run(&request, out.as_deref())
            .map_err(|error| Failure::Runtime(error.to_string())),
    }
}
