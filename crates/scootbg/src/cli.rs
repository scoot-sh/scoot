//! Argument parsing, hand-rolled (dependencies-done.md §4: +12 KB against
//! +299 KB for `clap`). One binary: `daemon` runs the Wayland client, and
//! every other command is a client of its control socket.
//!
//! `apply-config` arrives with cli-and-ipc.md, and image paths for `set`
//! with images; neither is parsed nor advertised until it works.

use std::borrow::Cow;
use std::ffi::OsString;
use std::fmt;

use crate::color::{Color, ColorError};
use crate::protocol::{PROTOCOL_VERSION, Request};

#[cfg(test)]
mod tests;

pub const USAGE: &str = "\
scootbg -- wallpaper daemon for Wayland

Early days: solid colors work; images come in a later version.

USAGE:
    scootbg COMMAND
    scootbg COMMAND --help
    scootbg --version
    scootbg --help

COMMANDS:
    daemon     run the daemon for this Wayland display
    set        show a color on every output, or on one
    clear      back to the compositor's own background
    query      print what each output shows, as JSON
    version    print the running daemon's version and protocol, as JSON
    kill       stop the running daemon

The daemon listens on $XDG_RUNTIME_DIR/scootbg-NAME.sock, where NAME is the
last component of $WAYLAND_DISPLAY (wayland-0 when unset); every other
command talks to it there.
";

pub const DAEMON_HELP: &str = "\
scootbg daemon -- run the wallpaper daemon

USAGE:
    scootbg daemon

Connects to the compositor named by $WAYLAND_DISPLAY, which must support
wlr-layer-shell, and serves requests on $XDG_RUNTIME_DIR/scootbg-NAME.sock.
Each output gets one background-layer surface, kept across outputs coming
and going; with no outputs at all the daemon waits for one.

Runs until `scootbg kill` (exit status 0) or until the compositor goes
away (exit status 1); either way the socket is removed. SIGTERM, SIGINT and
SIGHUP end it at once and leave the socket file behind; that is harmless:
other commands then report no daemon running, and the next `scootbg daemon`
replaces the file.

One daemon per display: a second one exits with an error while the first
is alive. A socket left behind by a daemon that crashed is replaced.
";

pub const SET_HELP: &str = "\
scootbg set -- show a color on every output, or on one

USAGE:
    scootbg set '#rrggbb'
    scootbg set '#rrggbb' --output NAME

A color is '#' and six hex digits, either case, such as '#1e1e2e'; quote
it, since the shell reads '#' as a comment. Anything not starting with '#'
is refused for now: images come in a later version. Wallpapers are opaque,
so there is no alpha.

Without --output, every output shows the color, including outputs plugged
in later, and any choice made for a single output is replaced. With
--output NAME (a connector name, as `scootbg query` lists them), only that
output does, and it keeps the color when it is unplugged and plugged back
in. A name that no output has now is an error, and nothing is changed.

Returns once every targeted output shows the color and the compositor has
processed it, so a screenshot taken straight after shows it. An output
unplugged meanwhile is left out of that wait; an output whose surface is
not configured yet is waited for. Prints nothing on success.

Exit status: 0 once shown; 1 when no daemon is running, the output is
unknown, or drawing failed (the daemon's stderr says why); 2 for a usage
error, such as a malformed color.
";

pub const CLEAR_HELP: &str = "\
scootbg clear -- back to the compositor's own background

USAGE:
    scootbg clear
    scootbg clear --output NAME

Takes the wallpaper off every output (without --output, including any
choice made for a single output), or off the output named NAME, so the
compositor's own background shows. The daemon keeps running. An output
plugged in later shows nothing until the next `scootbg set`, unless a
color was set for it by name.

Returns once the compositor has processed it, like `set`, and prints
nothing on success. Exit status as for `set`.
";

pub const QUERY_HELP: &str = "\
scootbg query -- print what each output shows

USAGE:
    scootbg query

Prints the daemon's reply, one line of JSON with one entry per output:
{\"type\":\"outputs\",\"outputs\":[...]}
Each entry has the output's name, description, mode, scale, transform and
logical size, its surface's state: waiting, pending, configured (with its
size), closed or gave-up (until the output is replugged), and what it
shows: {\"color\":\"#rrggbb\"}, or null for nothing.
";

pub const VERSION_HELP: &str = "\
scootbg version -- ask the running daemon for its version

USAGE:
    scootbg version

Prints the daemon's reply, one line of JSON:
{\"type\":\"version\",\"protocol\":N,\"version\":\"X.Y.Z\"}
`scootbg --version` prints this binary's own version without a daemon.
";

pub const KILL_HELP: &str = "\
scootbg kill -- stop the running daemon

USAGE:
    scootbg kill

Returns once the daemon has removed its socket and closed the connection,
so a new `scootbg daemon` can start straight after.

Exit status: 0 once the daemon has stopped; 1 with \"no scootbg daemon is
running\" when none answers (none started, or it died and left its socket
file behind); 1 for any other error.
";

/// Which help text to print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topic {
    Main,
    Daemon,
    Set,
    Clear,
    Query,
    Version,
    Kill,
}

impl Topic {
    pub fn text(self) -> &'static str {
        match self {
            Self::Main => USAGE,
            Self::Daemon => DAEMON_HELP,
            Self::Set => SET_HELP,
            Self::Clear => CLEAR_HELP,
            Self::Query => QUERY_HELP,
            Self::Version => VERSION_HELP,
            Self::Kill => KILL_HELP,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help(Topic),
    /// `--version`: this binary, answered locally.
    Version,
    Daemon,
    /// A request for the running daemon.
    Client(Request<'static>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Missing,
    Unknown(String),
    Unexpected {
        command: &'static str,
        argument: String,
    },
    /// `set` with no color.
    MissingColor,
    /// `set` with something that is not a color.
    Color {
        argument: String,
        error: ColorError,
    },
    /// A flag given without its value.
    MissingValue {
        command: &'static str,
        flag: &'static str,
    },
    /// A flag given twice.
    Repeated {
        command: &'static str,
        flag: &'static str,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(f, "missing command (try --help)"),
            Self::Unknown(what) => write!(f, "unknown command `{what}` (try --help)"),
            Self::Unexpected { command, argument } => write!(
                f,
                "unexpected argument `{argument}` for `{command}` (try `scootbg {command} --help`)"
            ),
            Self::MissingColor => write!(
                f,
                "`set` needs a color, such as '#1e1e2e' (try `scootbg set --help`)"
            ),
            Self::Color { argument, error } => {
                write!(f, "`{argument}`: {error} (try `scootbg set --help`)")
            }
            Self::MissingValue { command, flag } => {
                write!(f, "`{flag}` needs a value (try `scootbg {command} --help`)")
            }
            Self::Repeated { command, flag } => {
                write!(f, "`{flag}` given twice (try `scootbg {command} --help`)")
            }
        }
    }
}

impl std::error::Error for Error {}

/// The `--version` line: this build and the protocol it speaks.
pub fn version_string() -> String {
    format!(
        "scootbg {} (protocol {PROTOCOL_VERSION})",
        env!("CARGO_PKG_VERSION")
    )
}

fn is_help(arg: &str) -> bool {
    matches!(arg, "--help" | "-h")
}

/// Parses the arguments after the program name. Takes `OsString`s so a
/// non-UTF-8 argument is an error, not a panic.
pub fn parse<I: IntoIterator<Item = OsString>>(args: I) -> Result<Command, Error> {
    let mut args = args.into_iter().map(|arg| {
        arg.into_string()
            .map_err(|raw| raw.to_string_lossy().into_owned())
    });
    let first = match args.next() {
        None => return Err(Error::Missing),
        Some(Err(lossy)) => return Err(Error::Unknown(lossy)),
        Some(Ok(first)) => first,
    };
    let (command, topic): (Command, Topic) = match first.as_str() {
        "--help" | "-h" | "help" => return help(args),
        "--version" | "-V" => (Command::Version, Topic::Main),
        "daemon" => (Command::Daemon, Topic::Daemon),
        "set" => return change("set", Topic::Set, args),
        "clear" => return change("clear", Topic::Clear, args),
        "query" => (Command::Client(Request::Query), Topic::Query),
        "version" => (Command::Client(Request::Version), Topic::Version),
        "kill" => (Command::Client(Request::Kill), Topic::Kill),
        _ => return Err(Error::Unknown(first)),
    };
    let name = match &command {
        Command::Version => "--version",
        Command::Daemon => "daemon",
        Command::Client(request) => request.name(),
        Command::Help(_) => "help",
    };
    match args.next() {
        None => Ok(command),
        Some(Ok(arg)) if is_help(&arg) && command != Command::Version => match args.next() {
            None => Ok(Command::Help(topic)),
            Some(extra) => Err(Error::Unexpected {
                command: name,
                argument: extra.unwrap_or_else(|lossy| lossy),
            }),
        },
        Some(other) => Err(Error::Unexpected {
            command: name,
            argument: other.unwrap_or_else(|lossy| lossy),
        }),
    }
}

/// `help` / `--help`, optionally followed by one command name.
fn help<I: Iterator<Item = Result<String, String>>>(mut args: I) -> Result<Command, Error> {
    let topic = match args.next() {
        None => return Ok(Command::Help(Topic::Main)),
        Some(Ok(name)) => match name.as_str() {
            "daemon" => Topic::Daemon,
            "set" => Topic::Set,
            "clear" => Topic::Clear,
            "query" => Topic::Query,
            "version" => Topic::Version,
            "kill" => Topic::Kill,
            _ => return Err(Error::Unknown(name)),
        },
        Some(Err(lossy)) => return Err(Error::Unknown(lossy)),
    };
    match args.next() {
        None => Ok(Command::Help(topic)),
        Some(extra) => Err(Error::Unexpected {
            command: "help",
            argument: extra.unwrap_or_else(|lossy| lossy),
        }),
    }
}

/// `set COLOR [--output NAME]` and `clear [--output NAME]`, flags in any
/// order after the command; `--output=NAME` works too. `--help` alone asks
/// for help, as for every command.
fn change<I: Iterator<Item = Result<String, String>>>(
    command: &'static str,
    topic: Topic,
    mut args: I,
) -> Result<Command, Error> {
    const OUTPUT: &str = "--output";
    let unexpected = |argument: String| Error::Unexpected { command, argument };
    let mut color: Option<String> = None;
    let mut output: Option<String> = None;
    let mut first = true;
    while let Some(arg) = args.next() {
        let arg = arg.map_err(unexpected)?;
        let value = if arg == OUTPUT {
            Some(args.next().ok_or(Error::MissingValue {
                command,
                flag: OUTPUT,
            })?)
        } else {
            arg.strip_prefix("--output=").map(|v| Ok(v.to_owned()))
        };
        if let Some(value) = value {
            let value = value.map_err(unexpected)?;
            if output.replace(value).is_some() {
                return Err(Error::Repeated {
                    command,
                    flag: OUTPUT,
                });
            }
        } else if is_help(&arg) && first {
            return match args.next() {
                None => Ok(Command::Help(topic)),
                Some(extra) => Err(unexpected(extra.unwrap_or_else(|lossy| lossy))),
            };
        } else if command == "set" && color.is_none() && !arg.starts_with('-') {
            color = Some(arg);
        } else {
            return Err(unexpected(arg));
        }
        first = false;
    }
    let output = output.map(Cow::Owned);
    if command == "clear" {
        return Ok(Command::Client(Request::Clear { output }));
    }
    let argument = color.ok_or(Error::MissingColor)?;
    match Color::parse(&argument) {
        Ok(color) => Ok(Command::Client(Request::Set { color, output })),
        Err(error) => Err(Error::Color { argument, error }),
    }
}
