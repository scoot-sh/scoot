//! The command line: `scootlaunch --list [--all] [--lenient] [--json]`.
//!
//! Later tickets add `--query`, `--run-id`, the panel flags and the
//! dmenu contract (`dev/design/scootlaunch.md`, Modes). Anything not
//! listed here is a loud error (exit 2, naming the flag and the
//! supported list), never silently ignored.

use std::fmt;

#[cfg(test)]
mod tests;

/// What `--list` takes. `all` shows `NoDisplay` and
/// `OnlyShowIn`/`NotShowIn`/`TryExec`-hidden entries too, with their
/// reason; `json` prints the agent JSON schema instead of text lines;
/// `lenient` accepts what GLib accepts for Exec quoting (single
/// quotes, unquoted reserved characters) instead of the spec's strict
/// rule, for real files that break it.
#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub list: bool,
    pub all: bool,
    pub lenient: bool,
    pub json: bool,
    pub help: bool,
    pub version: bool,
}

pub const HELP: &str = "\
scootlaunch: a keyboard-first application launcher for Wayland

usage: scootlaunch --list [--all] [--lenient] [--json]

  --list       list indexed desktop entries (no compositor needed)
  --all        include hidden entries, with their reason
  --lenient    Exec quoting: accept what GLib accepts, not just the spec
  --json       print the agent JSON schema (id, name, exec argv, ...) array
  --help       print this text
  --version    print the version
";

/// A usage error. Printed by the caller; exit status 2.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// No command at all (`scootlaunch` with no flags).
    NoCommand,
    /// A flag this ticket does not take, with the supported list.
    UnknownFlag(String),
    /// A positional argument, which no command takes.
    UnexpectedArg(String),
    /// `--all`, `--lenient` or `--json` without `--list`.
    NeedsList(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoCommand => write!(
                f,
                "no command: usage is scootlaunch --list [--all] [--lenient] [--json]"
            ),
            Error::UnknownFlag(flag) => write!(
                f,
                "unknown flag {flag}: supported flags are --list --all --lenient --json --help --version"
            ),
            Error::UnexpectedArg(arg) => write!(
                f,
                "unexpected argument {arg}: usage is scootlaunch --list [--all] [--lenient] [--json]"
            ),
            Error::NeedsList(flag) => {
                write!(
                    f,
                    "{flag} needs --list: usage is scootlaunch --list [--all] [--lenient] [--json]"
                )
            }
        }
    }
}

/// Parses `argv` (including `argv[0]`, the program name). Every flag is
/// a whole argument; `--flag=value` is not a flag but an unknown one.
pub fn parse_args(argv: &[String]) -> Result<Args, Error> {
    let mut args = Args {
        list: false,
        all: false,
        lenient: false,
        json: false,
        help: false,
        version: false,
    };
    if argv.len() < 2 {
        return Err(Error::NoCommand);
    }
    // `--help` and `--version` win over every usage error below them,
    // wherever they sit: `scootlaunch --bogus --help` still prints
    // help, as any Unix tool does.
    for arg in &argv[1..] {
        match arg.as_str() {
            "--help" | "-h" => {
                return Ok(Args {
                    list: false,
                    all: false,
                    lenient: false,
                    json: false,
                    help: true,
                    version: false,
                });
            }
            "--version" | "-V" => {
                return Ok(Args {
                    list: false,
                    all: false,
                    lenient: false,
                    json: false,
                    help: false,
                    version: true,
                });
            }
            _ => {}
        }
    }
    for arg in &argv[1..] {
        match arg.as_str() {
            "--list" => args.list = true,
            "--all" => args.all = true,
            "--lenient" => args.lenient = true,
            "--json" => args.json = true,
            // Reached only when the pre-scan above did not fire, so
            // these arms never run; kept explicit so a future flag
            // reordering cannot route `--help` into the unknown-flag
            // error below.
            "--help" | "-h" => args.help = true,
            "--version" | "-V" => args.version = true,
            flag if flag.starts_with('-') => return Err(Error::UnknownFlag(flag.to_string())),
            other => return Err(Error::UnexpectedArg(other.to_string())),
        }
    }
    for flag in ["--all", "--lenient", "--json"] {
        let set = match flag {
            "--all" => args.all,
            "--lenient" => args.lenient,
            _ => args.json,
        };
        if set && !args.list {
            return Err(Error::NeedsList(flag.to_string()));
        }
    }
    if !args.list {
        return Err(Error::NoCommand);
    }
    Ok(args)
}
