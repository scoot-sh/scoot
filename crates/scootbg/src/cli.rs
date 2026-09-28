//! Argument parsing, hand-rolled (dependencies-done.md §4: +12 KB against
//! +299 KB for `clap`). One binary: `daemon` runs the Wayland client, and
//! every other command is a client of its control socket.
//!
//! `apply-config` is scoot's one command
//! (docs/scootbg/backlog/resolved/scoot-integration-done.md); its `--serve`
//! flag is internal, the detached daemon it starts (`crate::apply`), and is
//! not advertised.

use std::borrow::Cow;
use std::ffi::OsString;
use std::fmt;

use crate::color::{Color, ColorError};
use crate::image::{Filter, Mode};
use crate::protocol::{DEFAULT_FILL, ImageRequest, PROTOCOL_VERSION, Request, Show};
use crate::section::{Section, SectionError};
use crate::state::{Profile, ProfileError};

#[cfg(test)]
mod tests;

pub const USAGE: &str = "\
scootbg -- wallpaper daemon for Wayland

Early days: colors and images (PNG, JPEG, WebP) work, and the daemon
shows the last ones again when it next starts.

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
    apply-config
               apply scoot's [wallpaper] section, starting the daemon if
               none runs (what scoot runs; see `scootbg apply-config --help`)

The daemon listens on $XDG_RUNTIME_DIR/scootbg-NAME.sock, where NAME is the
last component of $WAYLAND_DISPLAY (wayland-0 when unset); every other
command talks to it there.
";

pub const DAEMON_HELP: &str = "\
scootbg daemon -- run the wallpaper daemon

USAGE:
    scootbg daemon [--profile NAME] [--no-restore]

Connects to the compositor named by $WAYLAND_DISPLAY, which must support
wlr-layer-shell, and serves requests on $XDG_RUNTIME_DIR/scootbg-NAME.sock.
Each output gets one background-layer surface, kept across outputs coming
and going; with no outputs at all the daemon waits for one.

Every `set` and `clear` is saved, per output, in the profile's state file,
$XDG_STATE_HOME/scootbg/PROFILE (~/.local/state/scootbg/PROFILE when
XDG_STATE_HOME is unset, empty or relative), and the daemon shows it again
when it next starts. If there is no state directory to use, or the file
cannot be used (unreadable, or written by a newer scootbg), saving is off
until the daemon restarts, stderr says how to recover, and `scootbg query`
reports \"saving\":false. A saved image that is gone (moved, deleted) is
skipped with a warning on stderr, and that output shows the compositor's
own background; the daemon starts all the same, and the entry stays saved
until a `set` or `clear` replaces it. Choices for outputs that are not
plugged in stay saved too.

    --profile NAME  which state to restore and save (default: default).
                    Sessions with different profiles never restore each
                    other's wallpaper; two sessions sharing one share it,
                    the last change winning. NAME is 1 to 64 of A-Z, a-z,
                    0-9, '.', '_' and '-', not starting with '.' and
                    without '..'
    --no-restore    start with nothing shown; the state is still read, and
                    a `set` or `clear` then updates it as usual

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
kept in memory. EXIF orientation is applied (a JPEG's, a WebP's, or a
PNG's eXIf chunk).

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
not configured yet is waited for, until a round trip after scootbg made
it (one the compositor is slower than that to configure is drawn when it
is, and does not hold up the reply); an output scootbg gave up on (`gave-up`
in `scootbg query`, said on stderr) is left out, shows nothing, and does
not change the exit status. When a newer `set` or `clear` replaces the
choice before this image is shown, this one changes nothing (it may never
be decoded) and returns with status 0 once the newer one is shown, as a
replaced color does. Prints nothing on success.

The choice is saved and shown again when the daemon next starts (see
`scootbg daemon --help`): a color at once, an image once it has decoded.

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
nothing on success. Saved like a `set`, so the daemon starts with it
cleared. Exit status as for `set`.
";

pub const QUERY_HELP: &str = "\
scootbg query -- print what each output shows

USAGE:
    scootbg query

Prints the daemon's reply, one line of JSON with one entry per output:
{\"type\":\"outputs\",\"outputs\":[...],\"saving\":true,\"profile\":\"default\"}
Each entry has the output's name, description, mode, scale, transform and
logical size, its surface's state: waiting, pending, configured (with its
size), closed or gave-up (until the output is replugged), whether drawing
what it should show failed (draw_failed; the daemon's stderr says why),
and what it shows: {\"color\":\"#rrggbb\"}, {\"image\":\"/path\",
\"mode\":\"fill\",\"fill\":\"#rrggbb\",\"filter\":\"lanczos3\"}, or null
for nothing. After the list, \"saving\" says whether changes are saved for
the next start (see `scootbg daemon --help`), and \"profile\" whose state
is restored and saved: the daemon's --profile, or the last one an
`apply-config` made it adopt.
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

pub const APPLY_CONFIG_HELP: &str = "\
scootbg apply-config -- apply scoot's [wallpaper] section

USAGE:
    scootbg apply-config [--profile NAME] JSON

What scoot runs at start-up and on every reload while its config has a
[wallpaper] section; you rarely need it yourself. JSON is the section, as
one object ('{}' when the section is gone):

    {\"image\": \"/abs/path.jpg\", \"mode\": \"fill\",
     \"output\": {\"DP-2\": {\"color\": \"#101014\"}},
     \"command\": \"scootbg\"}

At the top, the wallpaper for every output: \"image\" (an absolute path)
or \"color\" (\"#rrggbb\"), never both, and for an image \"mode\", \"fill\"
and \"filter\", as `scootbg set` takes them; neither is nothing (the
compositor's own background). \"output\" holds per-output tables by
connector name, each the same keys; an empty one means nothing there. Each
table stands alone: an output's image does not take the top level's mode.
\"command\" is scoot's, and ignored. Anything else is refused: an unknown
key, a key given twice, null, a relative path, a malformed color, an
unknown mode or filter, more than 256 outputs, over 63 KiB.

Whichever you changed last wins: the section is applied only if it changed
since the last apply-config for this profile (a fingerprint of it is kept
in the profile's state, see `scootbg daemon --help`). Unchanged, what
shows stays, so a `scootbg set` made since survives restarts and reloads
until you next change the section itself. \"command\" is not part of the
fingerprint.

When a daemon answers, it is sent the section and adopts --profile NAME
(default: default): from then on it restores and saves that profile's
state, whatever profile it started with. When none does, apply-config
starts one, detached (its own session, its stderr this command's), with
the section as its starting point, then sends it the section as above.
Two started at once settle it by the daemon's lock: one daemon runs, and
both sections reach it. With no daemon and an empty section ('{}', or only
\"command\"), it starts none: it records the clear in the profile's state
file, so a later `scootbg daemon --profile NAME` shows nothing.

A daemon from another scootbg build is reported on stderr: a different
version is a warning (the section is still sent); a different protocol, or
a daemon too old to know apply-config, is an error. A caller that reads
this command's stderr to its end waits for a daemon it started too, which
writes there: send stderr to a file or a log, not a pipe read to the end.

Returns once every output shows what it should and the compositor has
processed it, as `set` does; prints nothing on success.

Exit status: 0 applied, or unchanged, and shown; 1 no daemon could be
started or reached within 5 s, no reply within 30 s, the daemon closed the
connection before answering, a daemon from another protocol or too old,
an image in the section that is not a file (the rest is applied; every
run says so until the file is back, and the first run after shows it,
unless a `scootbg set` has replaced it there since), drawing failed, or
the state file could not be written; 2 for a usage error, the section
refused above included.
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
    ApplyConfig,
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
            Self::ApplyConfig => APPLY_CONFIG_HELP,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help(Topic),
    /// `--version`: this binary, answered locally.
    Version,
    Daemon(DaemonOptions),
    /// A request for the running daemon.
    Client(Request<'static>),
    /// `apply-config` (`crate::apply`).
    ApplyConfig(ApplyOptions),
}

/// `scootbg apply-config`'s arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOptions {
    pub profile: Profile,
    pub section: Section,
    /// `--serve` (internal): this process is the detached daemon an
    /// `apply-config` started.
    pub serve: bool,
}

/// `scootbg daemon`'s flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonOptions {
    /// Whose state to restore and save (`crate::state`).
    pub profile: Profile,
    /// `--no-restore` makes it false.
    pub restore: bool,
}

impl Default for DaemonOptions {
    /// Plain `scootbg daemon`: the default profile, restored.
    fn default() -> Self {
        Self {
            profile: Profile::default(),
            restore: true,
        }
    }
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
    /// `--profile` with a name that cannot be one.
    Profile(ProfileError),
    /// `apply-config` without its JSON.
    MissingSection,
    /// `apply-config` with JSON that is not UTF-8.
    SectionNotUtf8,
    /// `apply-config` with a section that is refused.
    Section(SectionError),
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
            Self::Profile(error) => write!(f, "`{PROFILE}`: {error}"),
            Self::MissingSection => write!(
                f,
                "`apply-config` needs the [wallpaper] section as JSON, '{{}}' for none \
                 (try `scootbg apply-config --help`)"
            ),
            Self::SectionNotUtf8 => write!(f, "`apply-config`: the JSON is not valid UTF-8"),
            Self::Section(error) => write!(
                f,
                "apply-config: {error} (try `scootbg apply-config --help`)"
            ),
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
        "daemon" => return daemon(args),
        "set" => return change("set", Topic::Set, args),
        "clear" => return change("clear", Topic::Clear, args),
        "query" => (Command::Client(Request::Query), Topic::Query),
        "version" => (Command::Client(Request::Version), Topic::Version),
        "kill" => (Command::Client(Request::Kill), Topic::Kill),
        "apply-config" => return apply_config(args),
        _ => return Err(Error::Unknown(first)),
    };
    let name = match &command {
        Command::Version => "--version",
        Command::Daemon(_) => "daemon",
        Command::Client(request) => request.name(),
        Command::Help(_) => "help",
        Command::ApplyConfig(_) => "apply-config",
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
            "apply-config" => Topic::ApplyConfig,
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

const PROFILE: &str = "--profile";
const NO_RESTORE: &str = "--no-restore";

/// `daemon [--profile NAME] [--no-restore]`, flags in any order, `--profile`
/// also as `--profile=NAME`; `--help` alone asks for help.
fn daemon<I: Iterator<Item = Result<String, String>>>(mut args: I) -> Result<Command, Error> {
    const COMMAND: &str = "daemon";
    let unexpected = |argument: String| Error::Unexpected {
        command: COMMAND,
        argument,
    };
    let mut profile: Option<String> = None;
    let mut no_restore = false;
    let mut first = true;
    while let Some(arg) = args.next() {
        let arg = arg.map_err(unexpected)?;
        if is_help(&arg) && first {
            return match args.next() {
                None => Ok(Command::Help(Topic::Daemon)),
                Some(extra) => Err(unexpected(extra.unwrap_or_else(|lossy| lossy))),
            };
        }
        first = false;
        if arg == NO_RESTORE {
            if std::mem::replace(&mut no_restore, true) {
                return Err(Error::Repeated {
                    command: COMMAND,
                    flag: NO_RESTORE,
                });
            }
            continue;
        }
        let value = if arg == PROFILE {
            args.next()
                .ok_or(Error::MissingValue {
                    command: COMMAND,
                    flag: PROFILE,
                })?
                .map_err(unexpected)?
        } else if let Some(value) = arg.strip_prefix(PROFILE).and_then(|v| v.strip_prefix('=')) {
            value.to_owned()
        } else {
            return Err(unexpected(arg));
        };
        if profile.replace(value).is_some() {
            return Err(Error::Repeated {
                command: COMMAND,
                flag: PROFILE,
            });
        }
    }
    let profile = match profile {
        None => Profile::default(),
        Some(name) => Profile::parse(&name).map_err(Error::Profile)?,
    };
    Ok(Command::Daemon(DaemonOptions {
        profile,
        restore: !no_restore,
    }))
}

/// `apply-config`'s internal flag: this process is the detached daemon
/// (`crate::apply`).
pub const SERVE: &str = "--serve";

/// `apply-config [--profile NAME] JSON`, flags in any order, `--profile`
/// also as `--profile=NAME`; `--help` alone asks for help. `--serve` is
/// internal (see [`ApplyOptions::serve`]).
fn apply_config<I: Iterator<Item = Result<String, String>>>(mut args: I) -> Result<Command, Error> {
    const COMMAND: &str = "apply-config";
    let unexpected = |argument: String| Error::Unexpected {
        command: COMMAND,
        argument,
    };
    let mut profile: Option<String> = None;
    let mut json: Option<String> = None;
    let mut serve = false;
    let mut first = true;
    while let Some(arg) = args.next() {
        let arg = match arg {
            Ok(arg) => arg,
            Err(lossy) if json.is_none() && !lossy.starts_with('-') => {
                return Err(Error::SectionNotUtf8);
            }
            Err(lossy) => return Err(unexpected(lossy)),
        };
        if is_help(&arg) && first {
            return match args.next() {
                None => Ok(Command::Help(Topic::ApplyConfig)),
                Some(extra) => Err(unexpected(extra.unwrap_or_else(|lossy| lossy))),
            };
        }
        first = false;
        if arg == SERVE {
            if std::mem::replace(&mut serve, true) {
                return Err(Error::Repeated {
                    command: COMMAND,
                    flag: SERVE,
                });
            }
            continue;
        }
        let value = if arg == PROFILE {
            args.next()
                .ok_or(Error::MissingValue {
                    command: COMMAND,
                    flag: PROFILE,
                })?
                .map_err(unexpected)?
        } else if let Some(value) = arg.strip_prefix(PROFILE).and_then(|v| v.strip_prefix('=')) {
            value.to_owned()
        } else if json.is_none() && !arg.starts_with('-') {
            json = Some(arg);
            continue;
        } else {
            return Err(unexpected(arg));
        };
        if profile.replace(value).is_some() {
            return Err(Error::Repeated {
                command: COMMAND,
                flag: PROFILE,
            });
        }
    }
    let profile = match profile {
        None => Profile::default(),
        Some(name) => Profile::parse(&name).map_err(Error::Profile)?,
    };
    let json = json.ok_or(Error::MissingSection)?;
    let section = Section::parse(json.as_bytes()).map_err(Error::Section)?;
    Ok(Command::ApplyConfig(ApplyOptions {
        profile,
        section,
        serve,
    }))
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
