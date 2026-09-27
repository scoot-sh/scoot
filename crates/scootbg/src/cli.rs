//! Argument parsing, hand-rolled (dependencies-done.md §4: +12 KB against
//! +299 KB for `clap`). One binary: `daemon` runs the Wayland client, and
//! every other command is a client of its control socket.
//!
//! `apply-config` arrives with the scoot integration (scoot-integration.md);
//! it is neither parsed nor advertised until it works.

use std::borrow::Cow;
use std::ffi::OsString;
use std::fmt;

use crate::color::{Color, ColorError};
use crate::image::{Filter, Mode};
use crate::protocol::{DEFAULT_FILL, ImageRequest, PROTOCOL_VERSION, Request, Show};

#[cfg(test)]
mod tests;

pub const USAGE: &str = "\
scootbg -- wallpaper daemon for Wayland

Early days: colors and images (PNG, JPEG, WebP) work; restoring them at
the next start comes later.

USAGE:
    scootbg COMMAND
    scootbg COMMAND --help
    scootbg --version
    scootbg --help

COMMANDS:
    daemon     run the daemon for this Wayland display
    set        show a color or an image on every output, or on one
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
scootbg set -- show a color or an image on every output, or on one

USAGE:
    scootbg set '#rrggbb' [--output NAME]
    scootbg set PATH [--output NAME] [--mode MODE] [--fill '#rrggbb']
                     [--filter FILTER]

An argument starting with '#' is a color: '#' and six hex digits, either
case, such as '#1e1e2e'; quote it, since the shell reads '#' as a comment.
Wallpapers are opaque, so there is no alpha.

Anything else is the path of an image: PNG, JPEG or WebP (the first frame
of an animated one), told apart by content, not by name. It is made
absolute here, so a relative path means from this directory. A file whose
name starts with '#' is given as './#name.png'. The file is read when the
daemon gets the request (and again for an output plugged in later), not
kept in memory. A JPEG's or WebP's EXIF orientation is applied.

    --mode MODE      how the image fits the output:
                       fill     cover it, cropping what overflows, centred
                                (the default)
                       fit      all of it, as large as fits, centred, the
                                rest in the fill color
                       stretch  the output's size, whatever the aspect
                       center   unscaled, centred: cropped if larger, the
                                rest in the fill color
                       tile     unscaled, repeated from the top-left corner
    --fill '#rrggbb' the color around a fitted or centred image, and under
                     a transparent one (default '#000000')
    --filter FILTER  the scaling filter: lanczos3 (the default), catmull-rom,
                     bilinear or nearest (hard pixels, for pixel art)

Without --output, every output shows it, including outputs plugged in
later, and any choice made for a single output is replaced. With --output
NAME (a connector name, as `scootbg query` lists them), only that output
does, and it keeps it when it is unplugged and plugged back in. A name that
no output has now is an error, and nothing is changed.

Returns once every targeted output shows it and the compositor has
processed it, so a screenshot taken straight after shows it. An image that
cannot be shown (no such file, not an image, too large, truncated or
corrupt) is an error, and every output keeps what it showed. An output
unplugged meanwhile is left out of that wait; an output whose surface is
not configured yet is waited for; an output scootbg gave up on (`gave-up`
in `scootbg query`, said on stderr) is left out, shows nothing, and does
not change the exit status. When a newer `set` or `clear` has replaced the
choice before this image was decoded, this one returns at once, with
status 0, and the newer one is what shows. Prints nothing on success.

Exit status: 0 once shown; 1 when no daemon is running, the output is
unknown, the image cannot be shown, or drawing failed (the daemon's stderr
says why); 2 for a usage error, such as a malformed color or an unknown
mode.
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
shows: {\"color\":\"#rrggbb\"}, {\"image\":\"/path\",\"mode\":\"fill\",
\"fill\":\"#rrggbb\",\"filter\":\"lanczos3\"}, or null for nothing.
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
    /// `set` with neither a color nor a path.
    MissingTarget,
    /// `set` with something that is not a color.
    Color {
        argument: String,
        error: ColorError,
    },
    /// `--mode` or `--filter` with a value it does not take.
    BadValue {
        flag: &'static str,
        value: String,
    },
    /// `--mode`, `--fill` or `--filter` with a color.
    ImageOnly(&'static str),
    /// A path that the control protocol (JSON) cannot carry.
    NotUtf8(String),
    /// The path could not be made absolute (no working directory).
    Path {
        argument: String,
        error: String,
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
            Self::MissingTarget => write!(
                f,
                "`set` needs a color, such as '#1e1e2e', or an image path \
                 (try `scootbg set --help`)"
            ),
            Self::Color { argument, error } => {
                write!(f, "`{argument}`: {error} (try `scootbg set --help`)")
            }
            Self::BadValue { flag, value } => {
                let takes = if *flag == MODE {
                    "fill, fit, stretch, center or tile"
                } else {
                    "lanczos3, catmull-rom, bilinear or nearest"
                };
                write!(
                    f,
                    "`{flag}` takes {takes}, not `{value}` (try `scootbg set --help`)"
                )
            }
            Self::ImageOnly(flag) => write!(
                f,
                "`{flag}` applies to an image, not a color (try `scootbg set --help`)"
            ),
            Self::NotUtf8(lossy) => write!(
                f,
                "`{lossy}`: the path is not valid UTF-8, which the control protocol cannot \
                 carry; rename the file or link to it from a UTF-8 path"
            ),
            Self::Path { argument, error } => {
                write!(f, "`{argument}`: cannot make the path absolute: {error}")
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

const OUTPUT: &str = "--output";
const MODE: &str = "--mode";
const FILL: &str = "--fill";
const FILTER: &str = "--filter";

/// `set COLOR|PATH [--output NAME] [--mode M] [--fill C] [--filter F]` and
/// `clear [--output NAME]`, flags in any order after the command, each
/// also as `--flag=VALUE`. `--help` alone asks for help, as for every
/// command.
fn change<I: Iterator<Item = Result<String, String>>>(
    command: &'static str,
    topic: Topic,
    mut args: I,
) -> Result<Command, Error> {
    let flags: &[&'static str] = if command == "set" {
        &[OUTPUT, MODE, FILL, FILTER]
    } else {
        &[OUTPUT]
    };
    let unexpected = |argument: String| Error::Unexpected { command, argument };
    let mut target: Option<String> = None;
    // Indexed as `flags`.
    let mut values: [Option<String>; 4] = Default::default();
    let mut first = true;
    while let Some(arg) = args.next() {
        let arg = match arg {
            Ok(arg) => arg,
            // Not UTF-8: as the path, say why it cannot be sent.
            Err(lossy) if command == "set" && target.is_none() && !lossy.starts_with('-') => {
                return Err(Error::NotUtf8(lossy));
            }
            Err(lossy) => return Err(unexpected(lossy)),
        };
        let flag = flags.iter().enumerate().find_map(|(index, &flag)| {
            if arg == flag {
                Some((index, flag, None))
            } else {
                let value = arg.strip_prefix(flag)?.strip_prefix('=')?;
                Some((index, flag, Some(value.to_owned())))
            }
        });
        if let Some((index, flag, value)) = flag {
            let value = match value {
                Some(value) => value,
                None => args
                    .next()
                    .ok_or(Error::MissingValue { command, flag })?
                    .map_err(unexpected)?,
            };
            let Some(slot) = values.get_mut(index) else {
                return Err(unexpected(arg));
            };
            if slot.replace(value).is_some() {
                return Err(Error::Repeated { command, flag });
            }
        } else if is_help(&arg) && first {
            return match args.next() {
                None => Ok(Command::Help(topic)),
                Some(extra) => Err(unexpected(extra.unwrap_or_else(|lossy| lossy))),
            };
        } else if command == "set" && target.is_none() && !arg.starts_with('-') {
            target = Some(arg);
        } else {
            return Err(unexpected(arg));
        }
        first = false;
    }
    let [output, mode, fill, filter] = values;
    let output = output.map(Cow::Owned);
    if command == "clear" {
        return Ok(Command::Client(Request::Clear { output }));
    }
    let argument = target.ok_or(Error::MissingTarget)?;
    if argument.starts_with('#') {
        for (flag, given) in [(MODE, &mode), (FILL, &fill), (FILTER, &filter)] {
            if given.is_some() {
                return Err(Error::ImageOnly(flag));
            }
        }
        return match Color::parse(&argument) {
            Ok(color) => Ok(Command::Client(Request::Set {
                show: Show::Color(color),
                output,
            })),
            Err(error) => Err(Error::Color { argument, error }),
        };
    }
    let mode = match mode {
        None => Mode::default(),
        Some(value) => Mode::from_name(&value).ok_or(Error::BadValue { flag: MODE, value })?,
    };
    let filter = match filter {
        None => Filter::default(),
        Some(value) => Filter::from_name(&value).ok_or(Error::BadValue {
            flag: FILTER,
            value,
        })?,
    };
    let fill = match fill {
        None => DEFAULT_FILL,
        Some(argument) => match Color::parse(&argument) {
            Ok(color) => color,
            Err(error) => return Err(Error::Color { argument, error }),
        },
    };
    let path = absolute(&argument)?;
    Ok(Command::Client(Request::Set {
        show: Show::Image(ImageRequest {
            path: Cow::Owned(path),
            mode,
            fill,
            filter,
        }),
        output,
    }))
}

/// `path` made absolute against the working directory, without touching
/// the file system beyond that (symbolic links stay as given; the daemon
/// reports a missing file).
fn absolute(path: &str) -> Result<String, Error> {
    let absolute = std::path::absolute(path).map_err(|error| Error::Path {
        argument: path.to_owned(),
        error: error.to_string(),
    })?;
    absolute
        .into_os_string()
        .into_string()
        .map_err(|lossy| Error::NotUtf8(lossy.to_string_lossy().into_owned()))
}
