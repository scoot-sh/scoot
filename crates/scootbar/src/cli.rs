//! Argument parsing, hand-rolled as scootbg's is (its dependency record,
//! §4: +12 KB against +299 KB for `clap`). The bar has no config file yet
//! (`docs/scootbar/backlog/config-cli-and-reload.md` brings one), so every
//! option is a flag on `daemon` with a fixed default; the flags stay as
//! overrides once the file exists.

use std::ffi::OsString;
use std::fmt;

use crate::bar::{self, Bar, Edge, MAX_HEIGHT, Margin, MarginError};
use crate::color::{Color, ColorError};

#[cfg(test)]
mod tests;

pub const USAGE: &str = "\
scootbar -- status bar for Wayland

Early days: a solid bar on every output, reserving its space; the clock
and modules come next.

USAGE:
    scootbar daemon [OPTIONS]
    scootbar daemon --help
    scootbar --version
    scootbar --help

COMMANDS:
    daemon     run the bar on every output of this Wayland display
";

pub const DAEMON_HELP: &str = "\
scootbar daemon -- run the bar

USAGE:
    scootbar daemon [--edge EDGE] [--height N] [--margin M] [--background COLOR]

Connects to the compositor named by $WAYLAND_DISPLAY, which must support
wlr-layer-shell, and gives every output a bar: a top-layer surface along
one edge (namespace \"scootbar\") that reserves its height, so windows are
arranged beside it. Outputs plugged in later get one too, and an output
unplugged takes its bar with it; with no outputs at all it waits for one.
It draws at each output's real device pixels, fractional scales included,
and only when something changed: idle, it makes no system calls.

    --edge EDGE          top (the default) or bottom
    --height N           the bar's height in logical pixels, 1 to 1024
                         (default 28)
    --margin M           space between the bar and the output's edges, in
                         logical pixels, as in CSS: ALL, VERTICAL,HORIZONTAL,
                         TOP,HORIZONTAL,BOTTOM or TOP,RIGHT,BOTTOM,LEFT, each
                         0 to 1024 (default 0). Windows keep clear of the bar
                         and of the margin on its edge; the margin on the
                         opposite edge does nothing
    --background COLOR   the bar's color, '#rrggbb' (default '#1e1e2e');
                         quote it, since the shell reads '#' as a comment

Runs until the compositor goes away (exit status 1, saying why) or it is
killed; SIGTERM and SIGINT end it at once, which is harmless: it keeps no
state. The compositor removes the bars with the connection.
";

/// A help page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topic {
    Main,
    Daemon,
}

impl Topic {
    pub fn text(self) -> &'static str {
        match self {
            Self::Main => USAGE,
            Self::Daemon => DAEMON_HELP,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help(Topic),
    Version,
    Daemon(Bar),
}

/// The flags `daemon` takes, for messages.
const EDGE: &str = "--edge";
const HEIGHT: &str = "--height";
const MARGIN: &str = "--margin";
const BACKGROUND: &str = "--background";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Missing,
    Unknown(String),
    Unexpected {
        command: &'static str,
        argument: String,
    },
    MissingValue(&'static str),
    Repeated(&'static str),
    Edge(String),
    Height(String),
    Margin {
        value: String,
        error: MarginError,
    },
    Color {
        value: String,
        error: ColorError,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(f, "missing command (try --help)"),
            Self::Unknown(what) => write!(f, "unknown command `{what}` (try --help)"),
            Self::Unexpected { command, argument } => write!(
                f,
                "unexpected argument `{argument}` for `{command}` (try `scootbar {command} --help`)"
            ),
            Self::MissingValue(flag) => {
                write!(f, "`{flag}` needs a value (try `scootbar daemon --help`)")
            }
            Self::Repeated(flag) => {
                write!(f, "`{flag}` given twice (try `scootbar daemon --help`)")
            }
            Self::Edge(value) => write!(f, "`{EDGE}` takes top or bottom, not `{value}`"),
            Self::Height(value) => write!(
                f,
                "`{HEIGHT}` takes a whole number of logical pixels from 1 to {MAX_HEIGHT}, \
                 not `{value}`"
            ),
            Self::Margin { value, error } => write!(f, "`{MARGIN} {value}`: {error}"),
            Self::Color { value, error } => write!(f, "`{BACKGROUND} {value}`: {error}"),
        }
    }
}

impl std::error::Error for Error {}

/// The `--version` line.
pub fn version_string() -> String {
    format!("scootbar {}", env!("CARGO_PKG_VERSION"))
}

fn is_help(arg: &str) -> bool {
    matches!(arg, "--help" | "-h")
}

/// An argument as UTF-8, or its lossy form for an error message.
type Arg = Result<String, String>;

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
    match first.as_str() {
        "--help" | "-h" | "help" => help(args),
        "--version" | "-V" => match args.next() {
            None => Ok(Command::Version),
            Some(extra) => Err(Error::Unexpected {
                command: "--version",
                argument: extra.unwrap_or_else(|lossy| lossy),
            }),
        },
        "daemon" => daemon(args),
        _ => Err(Error::Unknown(first)),
    }
}

/// `help` / `--help`, optionally followed by one command name.
fn help(mut args: impl Iterator<Item = Arg>) -> Result<Command, Error> {
    let topic = match args.next() {
        None => Topic::Main,
        Some(Ok(name)) if name == "daemon" => Topic::Daemon,
        Some(other) => return Err(Error::Unknown(other.unwrap_or_else(|lossy| lossy))),
    };
    match args.next() {
        None => Ok(Command::Help(topic)),
        Some(extra) => Err(Error::Unexpected {
            command: "help",
            argument: extra.unwrap_or_else(|lossy| lossy),
        }),
    }
}

/// `daemon`'s flags: each at most once, as `--flag VALUE` or
/// `--flag=VALUE`; `--help` alone asks for its page.
fn daemon(mut args: impl Iterator<Item = Arg>) -> Result<Command, Error> {
    let mut edge: Option<Edge> = None;
    let mut height: Option<u32> = None;
    let mut margin: Option<Margin> = None;
    let mut background: Option<Color> = None;
    let mut first = true;
    while let Some(arg) = args.next() {
        let arg = arg.map_err(|lossy| Error::Unexpected {
            command: "daemon",
            argument: lossy,
        })?;
        if first && is_help(&arg) {
            return match args.next() {
                None => Ok(Command::Help(Topic::Daemon)),
                Some(extra) => Err(Error::Unexpected {
                    command: "daemon",
                    argument: extra.unwrap_or_else(|lossy| lossy),
                }),
            };
        }
        first = false;
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        let flag = match name {
            EDGE => EDGE,
            HEIGHT => HEIGHT,
            MARGIN => MARGIN,
            BACKGROUND => BACKGROUND,
            _ => {
                return Err(Error::Unexpected {
                    command: "daemon",
                    argument: arg,
                });
            }
        };
        let value = match inline {
            Some(value) => value,
            None => match args.next() {
                Some(Ok(value)) => value,
                // A value that is not UTF-8 cannot be any of these.
                Some(Err(lossy)) => lossy,
                None => return Err(Error::MissingValue(flag)),
            },
        };
        match flag {
            EDGE => set(
                &mut edge,
                flag,
                Edge::parse(&value).ok_or(Error::Edge(value)),
            )?,
            HEIGHT => set(
                &mut height,
                flag,
                bar::parse_height(&value).ok_or(Error::Height(value)),
            )?,
            MARGIN => set(
                &mut margin,
                flag,
                Margin::parse(&value).map_err(|error| Error::Margin { value, error }),
            )?,
            _ => set(
                &mut background,
                flag,
                Color::parse(&value).map_err(|error| Error::Color { value, error }),
            )?,
        }
    }
    let defaults = Bar::default();
    Ok(Command::Daemon(Bar {
        edge: edge.unwrap_or(defaults.edge),
        height: height.unwrap_or(defaults.height),
        margin: margin.unwrap_or(defaults.margin),
        background: background.unwrap_or(defaults.background),
    }))
}

/// Stores a flag's parsed value, refusing a repeat before a bad value (so
/// `--height 1 --height x` says "given twice", the first mistake made).
fn set<T>(slot: &mut Option<T>, flag: &'static str, value: Result<T, Error>) -> Result<(), Error> {
    if slot.is_some() {
        return Err(Error::Repeated(flag));
    }
    *slot = Some(value?);
    Ok(())
}
