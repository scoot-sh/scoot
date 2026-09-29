//! Argument parsing, hand-rolled as scootbg's is (its dependency record,
//! §4: +12 KB against +299 KB for `clap`). The bar has no config file yet
//! (`docs/scootbar/backlog/config-cli-and-reload.md` brings one), so every
//! option is a flag on `daemon` with a fixed default; the flags stay as
//! overrides once the file exists.

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use crate::bar::{self, Edge, MAX_HEIGHT, Margin, MarginError};
use crate::color::{Color, ColorError};
use crate::config::{Config, MAX_FONT_SIZE};
use crate::layout::{Layout, MAX_GAP, MAX_MODULES, Section};
use crate::modules::{self, REGISTRY};

#[cfg(test)]
mod tests;

// The help text is put together from pieces, so a build without a module
// documents only what it has: the pieces that name the clock come in two
// versions, one per build.
#[cfg(feature = "clock")]
macro_rules! what_it_shows {
    () => {
        "A bar on every output, reserving its space, with a clock."
    };
}
#[cfg(not(feature = "clock"))]
macro_rules! what_it_shows {
    () => {
        "A bar on every output, reserving its space (built with no modules)."
    };
}

#[cfg(feature = "clock")]
macro_rules! idle {
    () => {
        "and only when something changed. Idle, it wakes twice a minute: the
clock's tick, and the compositor's release of the frame the tick replaced
(one per frame, for any wl_shm client); with no module placed, never."
    };
}
#[cfg(not(feature = "clock"))]
macro_rules! idle {
    () => {
        "and only when something changed: idle, it never wakes."
    };
}

#[cfg(feature = "clock")]
macro_rules! modules {
    () => {
        "    --left IDS           the modules along the left, center and right,
    --center IDS         comma-separated, in order (default: the clock in
    --right IDS          the center). Giving any of the three sets the whole
                         layout: a section not given is empty. Modules: clock
    --padding N          logical pixels either side of each module, 0 to 1024
                         (default 8)
    --spacing N          logical pixels between neighbouring modules, 0 to
                         1024 (default 0)
    --clock-format FMT   the clock, as strftime (default '%-I:%M %P', which
                         shows 3:07 pm; '%H:%M' for 15:07). Specifiers:
                         %H %I %k %l %M %S %p %P %a %A %b %h %B %d %e %m %j
                         %y %Y %u %w %Z %z %R %T %F %D %%, and the flags -
                         (no padding), _ (spaces) and 0 (zeros). With %S or
                         %T it ticks every second, else once a minute. The
                         zone is $TZ's, else /etc/localtime's
"
    };
}
#[cfg(not(feature = "clock"))]
macro_rules! modules {
    () => {
        "    --left IDS           the modules along the left, center and right,
    --center IDS         comma-separated, in order. This build has none, so
    --right IDS          only an empty list is taken
    --padding N          logical pixels either side of each module, 0 to 1024
                         (default 8)
    --spacing N          logical pixels between neighbouring modules, 0 to
                         1024 (default 0)
"
    };
}

pub const USAGE: &str = concat!(
    "\
scootbar -- status bar for Wayland

",
    what_it_shows!(),
    "

USAGE:
    scootbar daemon [OPTIONS]
    scootbar daemon --help
    scootbar --version
    scootbar --help

COMMANDS:
    daemon     run the bar on every output of this Wayland display
"
);

pub const DAEMON_HELP: &str = concat!(
    "\
scootbar daemon -- run the bar

USAGE:
    scootbar daemon [OPTIONS]

Connects to the compositor named by $WAYLAND_DISPLAY, which must support
wlr-layer-shell, and gives every output a bar: a top-layer surface along
one edge (namespace \"scootbar\") that reserves its height, so windows are
arranged beside it. Outputs plugged in later get one too, and an output
unplugged takes its bar with it; with no outputs at all it waits for one.
It draws at each output's real device pixels, fractional scales included,
",
    idle!(),
    "

The bar:
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
    --foreground COLOR   the text's color (default '#cdd6f4')

Text:
    --font PATH          a .ttf or .otf file. Without it, the first of a few
                         well-known files (DejaVu Sans, Noto Sans) found; with
                         none, it refuses to start (a bar with no module
                         placed needs none)
    --font-size N        the text's size (the em) in logical pixels, 1 to 256
                         (default 14)

Modules:
",
    modules!(),
    "
Runs until the compositor goes away (exit status 1, saying why) or it is
killed; SIGTERM and SIGINT end it at once, which is harmless: it keeps no
state. The compositor removes the bars with the connection.
"
);

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
    Daemon(Box<Config>),
}

/// The flags `daemon` takes.
const EDGE: &str = "--edge";
const HEIGHT: &str = "--height";
const MARGIN: &str = "--margin";
const BACKGROUND: &str = "--background";
const FOREGROUND: &str = "--foreground";
const FONT: &str = "--font";
const FONT_SIZE: &str = "--font-size";
const LEFT: &str = "--left";
const CENTER: &str = "--center";
const RIGHT: &str = "--right";
const PADDING: &str = "--padding";
const SPACING: &str = "--spacing";
#[cfg(feature = "clock")]
const CLOCK_FORMAT: &str = "--clock-format";

const FLAGS: &[&str] = &[
    EDGE,
    HEIGHT,
    MARGIN,
    BACKGROUND,
    FOREGROUND,
    FONT,
    FONT_SIZE,
    LEFT,
    CENTER,
    RIGHT,
    PADDING,
    SPACING,
    #[cfg(feature = "clock")]
    CLOCK_FORMAT,
];

/// Why a module list is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModulesError {
    /// No module of that name in this build.
    Unknown(String),
    /// Placed twice (in one section or across two).
    Twice(&'static str),
    TooMany,
}

impl fmt::Display for ModulesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(id) => {
                write!(
                    f,
                    "no module `{}` in this build (it has:",
                    id.escape_debug()
                )?;
                if REGISTRY.is_empty() {
                    write!(f, " none")?;
                }
                for spec in REGISTRY {
                    write!(f, " {}", spec.id)?;
                }
                write!(f, ")")
            }
            Self::Twice(id) => write!(f, "`{id}` is placed twice; a module goes in one place"),
            Self::TooMany => write!(f, "at most {MAX_MODULES} modules"),
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
    MissingValue(&'static str),
    Repeated(&'static str),
    Edge(String),
    Height(String),
    Margin {
        value: String,
        error: MarginError,
    },
    Color {
        flag: &'static str,
        value: String,
        error: ColorError,
    },
    FontSize(String),
    Gap {
        flag: &'static str,
        value: String,
    },
    Modules {
        flag: &'static str,
        error: ModulesError,
    },
    #[cfg(feature = "clock")]
    ClockFormat {
        value: String,
        error: modules::clock::format::Error,
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
            Self::Color { flag, value, error } => write!(f, "`{flag} {value}`: {error}"),
            Self::FontSize(value) => write!(
                f,
                "`{FONT_SIZE}` takes a whole number of logical pixels from 1 to \
                 {MAX_FONT_SIZE}, not `{value}`"
            ),
            Self::Gap { flag, value } => write!(
                f,
                "`{flag}` takes a whole number of logical pixels from 0 to {MAX_GAP}, \
                 not `{value}`"
            ),
            Self::Modules { flag, error } => write!(f, "`{flag}`: {error}"),
            #[cfg(feature = "clock")]
            Self::ClockFormat { value, error } => {
                write!(f, "`{CLOCK_FORMAT} {}`: {error}", value.escape_debug())
            }
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
fn text(arg: OsString) -> Result<String, String> {
    arg.into_string()
        .map_err(|raw| raw.to_string_lossy().into_owned())
}

/// Parses the arguments after the program name. Takes `OsString`s so a
/// non-UTF-8 argument is an error (or, for `--font`, a path), not a panic.
pub fn parse<I: IntoIterator<Item = OsString>>(args: I) -> Result<Command, Error> {
    let mut args = args.into_iter();
    let first = match args.next().map(text) {
        None => return Err(Error::Missing),
        Some(Err(lossy)) => return Err(Error::Unknown(lossy)),
        Some(Ok(first)) => first,
    };
    match first.as_str() {
        "--help" | "-h" | "help" => help(args.map(text)),
        "--version" | "-V" => match args.next() {
            None => Ok(Command::Version),
            Some(extra) => Err(Error::Unexpected {
                command: "--version",
                argument: text(extra).unwrap_or_else(|lossy| lossy),
            }),
        },
        "daemon" => daemon(args),
        _ => Err(Error::Unknown(first)),
    }
}

/// `help` / `--help`, optionally followed by one command name.
fn help(mut args: impl Iterator<Item = Result<String, String>>) -> Result<Command, Error> {
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

/// Every `daemon` flag's value, as given (each at most once).
#[derive(Default)]
struct Given {
    edge: Option<Edge>,
    height: Option<u32>,
    margin: Option<Margin>,
    background: Option<Color>,
    foreground: Option<Color>,
    font: Option<PathBuf>,
    font_size: Option<u32>,
    left: Option<Vec<&'static str>>,
    center: Option<Vec<&'static str>>,
    right: Option<Vec<&'static str>>,
    padding: Option<u32>,
    spacing: Option<u32>,
    #[cfg(feature = "clock")]
    clock_format: Option<modules::clock::format::Format>,
}

/// `daemon`'s flags: each at most once, as `--flag VALUE` or
/// `--flag=VALUE`; `--help` alone asks for its page.
fn daemon(mut args: impl Iterator<Item = OsString>) -> Result<Command, Error> {
    let mut given = Given::default();
    let mut first = true;
    while let Some(arg) = args.next() {
        let arg = text(arg).map_err(|lossy| Error::Unexpected {
            command: "daemon",
            argument: lossy,
        })?;
        if first && is_help(&arg) {
            return match args.next() {
                None => Ok(Command::Help(Topic::Daemon)),
                Some(extra) => Err(Error::Unexpected {
                    command: "daemon",
                    argument: text(extra).unwrap_or_else(|lossy| lossy),
                }),
            };
        }
        first = false;
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(OsString::from(value))),
            _ => (arg.as_str(), None),
        };
        let Some(&flag) = FLAGS.iter().find(|&&flag| flag == name) else {
            return Err(Error::Unexpected {
                command: "daemon",
                argument: arg,
            });
        };
        let Some(raw) = inline.or_else(|| args.next()) else {
            return Err(Error::MissingValue(flag));
        };
        given.take(flag, raw)?;
    }
    Ok(Command::Daemon(Box::new(given.into_config()?)))
}

impl Given {
    /// Parses `raw` as `flag`'s value.
    fn take(&mut self, flag: &'static str, raw: OsString) -> Result<(), Error> {
        if flag == FONT {
            // A path: any bytes will do.
            return set(&mut self.font, flag, Ok(PathBuf::from(raw)));
        }
        // Every other flag takes text; a value that is not UTF-8 cannot be
        // any of them, and its lossy form says so in the flag's own error.
        let value = text(raw).unwrap_or_else(|lossy| lossy);
        match flag {
            EDGE => set(
                &mut self.edge,
                flag,
                Edge::parse(&value).ok_or(Error::Edge(value)),
            ),
            HEIGHT => set(
                &mut self.height,
                flag,
                bar::parse_height(&value).ok_or(Error::Height(value)),
            ),
            MARGIN => set(
                &mut self.margin,
                flag,
                Margin::parse(&value).map_err(|error| Error::Margin { value, error }),
            ),
            BACKGROUND | FOREGROUND => {
                let slot = if flag == BACKGROUND {
                    &mut self.background
                } else {
                    &mut self.foreground
                };
                set(
                    slot,
                    flag,
                    Color::parse(&value).map_err(|error| Error::Color { flag, value, error }),
                )
            }
            FONT_SIZE => set(
                &mut self.font_size,
                flag,
                bar::parse_whole(&value)
                    .filter(|size| (1..=MAX_FONT_SIZE).contains(size))
                    .ok_or(Error::FontSize(value)),
            ),
            PADDING | SPACING => {
                let slot = if flag == PADDING {
                    &mut self.padding
                } else {
                    &mut self.spacing
                };
                set(
                    slot,
                    flag,
                    bar::parse_whole(&value)
                        .filter(|gap| *gap <= MAX_GAP)
                        .ok_or(Error::Gap { flag, value }),
                )
            }
            #[cfg(feature = "clock")]
            CLOCK_FORMAT => set(
                &mut self.clock_format,
                flag,
                modules::clock::format::Format::parse(&value)
                    .map_err(|error| Error::ClockFormat { value, error }),
            ),
            // `--left`, `--center` and `--right`: `FLAGS` has nothing else.
            _ => {
                let slot = match flag {
                    LEFT => &mut self.left,
                    CENTER => &mut self.center,
                    _ => &mut self.right,
                };
                set(
                    slot,
                    flag,
                    module_list(&value).map_err(|error| Error::Modules { flag, error }),
                )
            }
        }
    }

    fn into_config(self) -> Result<Config, Error> {
        let defaults = Config::default();
        let padding = self.padding.unwrap_or(defaults.layout.padding);
        let spacing = self.spacing.unwrap_or(defaults.layout.spacing);
        let layout = if self.left.is_none() && self.center.is_none() && self.right.is_none() {
            Layout {
                padding,
                spacing,
                ..defaults.layout
            }
        } else {
            Layout {
                left: self.left.unwrap_or_default(),
                center: self.center.unwrap_or_default(),
                right: self.right.unwrap_or_default(),
                padding,
                spacing,
            }
        };
        check_layout(&layout)?;
        #[cfg_attr(not(feature = "clock"), allow(unused_mut))]
        let mut modules = defaults.modules;
        #[cfg(feature = "clock")]
        if let Some(format) = self.clock_format {
            modules.clock.format = format;
        }
        Ok(Config {
            bar: bar::Bar {
                edge: self.edge.unwrap_or(defaults.bar.edge),
                height: self.height.unwrap_or(defaults.bar.height),
                margin: self.margin.unwrap_or(defaults.bar.margin),
            },
            theme: crate::theme::Theme {
                background: self.background.unwrap_or(defaults.theme.background),
                foreground: self.foreground.unwrap_or(defaults.theme.foreground),
                ..defaults.theme
            },
            layout,
            font: self.font,
            font_size: self.font_size.unwrap_or(defaults.font_size),
            modules,
        })
    }
}

/// A comma-separated list of module ids; empty is no modules.
fn module_list(value: &str) -> Result<Vec<&'static str>, ModulesError> {
    let mut ids = Vec::new();
    if value.is_empty() {
        return Ok(ids);
    }
    for id in value.split(',') {
        let spec = modules::find(id).ok_or_else(|| ModulesError::Unknown(id.to_owned()))?;
        if ids.len() >= MAX_MODULES {
            return Err(ModulesError::TooMany);
        }
        ids.push(spec.id);
    }
    Ok(ids)
}

/// No module twice across the sections, and at most [`MAX_MODULES`].
fn check_layout(layout: &Layout) -> Result<(), Error> {
    let mut seen: Vec<&str> = Vec::new();
    for section in Section::ALL {
        for &id in layout.section(section) {
            let flag = section.flag();
            if seen.contains(&id) {
                return Err(Error::Modules {
                    flag,
                    error: ModulesError::Twice(id),
                });
            }
            if seen.len() >= MAX_MODULES {
                return Err(Error::Modules {
                    flag,
                    error: ModulesError::TooMany,
                });
            }
            seen.push(id);
        }
    }
    Ok(())
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
