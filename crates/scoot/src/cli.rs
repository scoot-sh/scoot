//! Argument parsing. Deliberately small: scoot's surface is one compositor to
//! start and a handful of requests to send.

use std::fmt;
use std::path::PathBuf;

use scoot_ipc::{Action, Horizontal, PointerButton, Request, Vertical};

pub const USAGE: &str = "\
scoot -- a scrolling-tiling Wayland compositor

USAGE:
    scoot --headless [--width 1-65535] [--height 1-65535] [--outputs 1-8] [--renderer pixman|gles] [--socket PATH] [--config PATH] [-- COMMAND...]
    scoot --nested [--width 1-65535] [--height 1-65535] [--renderer pixman|gles] [--socket PATH] [--config PATH] [-- COMMAND...]
    scoot --tty [--gpu PATH] [--mode WxH] [--socket PATH] [--config PATH] [-- COMMAND...]
    scoot msg REQUEST
    scoot --help

REQUESTS:
    version | outputs | windows
    action ACTION [ARGUMENT...]
    screenshot [--output ID] [--out FILE]
    pointer move X Y | pointer click X Y [left|right|middle]
    pointer button left|right|middle press|release | pointer scroll DX DY
    key COMBO                       e.g. Return, ctrl+shift+t -- name the key
                                    as it is unmodified plus the modifiers to
                                    hold (shift+1, not exclam)
    type TEXT                       types text, working out each character's
                                    own modifiers from the active layout
    wait-idle [--quiet-ms N] [--timeout-ms N]

ACTIONS:
    focus-column|move-column|consume-or-expel   left|right
    focus-window|move-window                    up|down
    focus-workspace|move-window-to-workspace    up|down
    focus-window-id ID | focus-workspace-index N | cycle-column-width | close | spawn COMMAND... | quit
";

/// The largest `--width`/`--height` a `--headless`/`--nested` output may ask
/// for, per axis.
///
/// Two reasons for this exact number, both about what a mode can report
/// rather than taste:
///
/// - **Nothing real can use more.** DRM reports each mode axis in a `u16`
///   (`drm_mode_modeinfo`'s `hdisplay`/`vdisplay` in the kernel's uAPI
///   headers), so no connector can list anything past 65535 -- and neither
///   can `--mode WxH`, which parses as `(u16, u16)` in this same file. Real
///   hardware sits far below that (8K is 7680 wide; the widest 16K
///   prototype is 15360), so the bound has room to spare with margin.
/// - **It keeps every output-derived sum in the layout far from overflow.**
///   The largest one, `available + gap` in `scoot_core`'s `column_width`,
///   tops out at `ceil(65535 / MIN_SCALE) + Config::MAX_GAP` -- 131070 +
///   10,000 at the `[output] scale` floor of 0.5 -- over four orders of
///   magnitude inside `i32`.
///
/// [`CompositorOptions::width`]/[`CompositorOptions::height`] below carry
/// the range into the type docs; `dimension` enforces it at parse.
pub const MAX_OUTPUT_DIMENSION: i32 = 65535;

/// The most outputs `--headless --outputs N` will create.
///
/// Small on purpose. The flag exists so multi-output behaviour is testable
/// with no second monitor in the building, and nothing tests more screens than
/// a desk holds -- an unbounded count would only buy a way to ask for a
/// million `wl_output` globals.
///
/// It also keeps the one sum the flag introduces far from overflow. Outputs are
/// laid out left to right, so the right edge of the last one is at most
/// `MAX_OUTPUTS * ceil(MAX_OUTPUT_DIMENSION / MIN_SCALE)` -- 8 * 131070, about
/// 1.05 million, four orders of magnitude inside `i32` and inside the same
/// margin [`MAX_OUTPUT_DIMENSION`] claims for the layout's own sums.
/// `headless::add_output` saturates in any case.
pub const MAX_OUTPUTS: i32 = 8;

/// Which renderer composites each frame.
///
/// Lives here rather than beside the renderers themselves because
/// [`CompositorOptions`] has to exist on every platform -- `scoot msg` builds
/// anywhere, while `compositor::render` is Linux-only -- and because the
/// config file parses the same two names (`[renderer] backend`). One name
/// list, one parser, the way [`action`] is shared with `[binds]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RendererKind {
    /// CPU compositing with pixman: the default, and the only renderer that
    /// needs no graphics device at all.
    #[default]
    Pixman,
    /// GLES on an EGL device. Opt-in, and `--headless`/`--nested` only
    /// today -- `--tty` scanout through a GPU is a separate piece of work,
    /// so `--tty` warns and keeps pixman (see `compositor::render::resolve`).
    Gles,
}

impl RendererKind {
    /// The spelling a user writes, in both the flag and the config file --
    /// and what every log line and warning names it by, so the two cannot
    /// drift.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pixman => "pixman",
            Self::Gles => "gles",
        }
    }

    /// Exactly the two names [`RendererKind::as_str`] produces, and nothing
    /// else -- no aliases, no case folding. `None` is "not one of ours",
    /// which the flag refuses outright and the config file warns about and
    /// ignores (see `compositor::config`).
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pixman" => Some(Self::Pixman),
            "gles" => Some(Self::Gles),
            _ => None,
        }
    }
}

impl fmt::Display for RendererKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, PartialEq)]
pub enum Command {
    Help,
    Compositor(CompositorOptions),
    Msg {
        request: Request,
        out: Option<PathBuf>,
    },
}

#[derive(Debug, PartialEq)]
pub struct CompositorOptions {
    /// The requested size. Under `--nested` it is only what scoot asks for:
    /// the host's first configure decides what the window comes up at, and
    /// every later one moves it (see `compositor::nested`). Under
    /// `--headless` it's authoritative, there being no host to negotiate
    /// with. Each axis is in `1..=MAX_OUTPUT_DIMENSION`; `parse` refuses
    /// anything else, and so does a host configure naming one.
    pub width: i32,
    pub height: i32,
    /// How many outputs `--headless` creates, each `width` by `height` and
    /// placed left to right with no gap. `1..=MAX_OUTPUTS`; `parse` refuses
    /// anything else.
    ///
    /// `--headless` only, and `compositor::run` warns and ignores it on the
    /// other two backends -- `--nested` presents one window in its host and
    /// `--tty` drives one CRTC, so neither has anywhere to put a second
    /// output. Exactly one output -- the first -- gets a render target; the
    /// rest exist as `wl_output` globals with their own geometry and their own
    /// scrolling strip in the core, which is what makes per-output protocol
    /// behaviour testable without a second monitor. Nothing is shown on a
    /// headless output in any case.
    pub outputs: i32,
    /// Where to listen for IPC; the default path when `None`.
    pub socket: Option<PathBuf>,
    /// Explicit config file path; the XDG default when `None`. See
    /// `compositor::config::load`'s doc for the resolution and fallback
    /// rules -- an explicit path here that can't be read is a hard startup
    /// error, unlike the default path.
    pub config: Option<PathBuf>,
    /// Launched once the compositor is up, with `WAYLAND_DISPLAY` set.
    pub command: Vec<String>,
    /// Present as a window inside the host compositor named by the *caller's*
    /// `WAYLAND_DISPLAY`, instead of running with no display at all.
    pub nested: bool,
    /// Present on a real DRM/KMS display via a Linux session (libseat) --
    /// the actual deployment target, rather than headless or nested inside
    /// another compositor. Mutually exclusive with `nested` (`parse` only
    /// ever sets one of the two). `width`/`height` are meaningless here and
    /// silently ignored: the connector's preferred mode (or `--mode`, see
    /// `mode` below) picks the output size, and unlike `--nested` there's no
    /// host to negotiate a different size with.
    pub tty: bool,
    /// `--tty`'s DRM device, when the automatic choice is wrong. `None`
    /// (the normal case) means `tty::gpu::candidates` picks: Smithay's
    /// `primary_gpu` first, then every other device on the seat as a
    /// fallback. A path here replaces that search entirely -- exactly one
    /// candidate, no fallback -- so a user on hardware both heuristics get
    /// wrong can name the right device instead. It also wins over the
    /// config file's `[tty] gpu` when both name one (an explicit flag beats
    /// a file, the way `--config` beats the default path). Meaningless outside
    /// `--tty`, where `compositor::run` ignores it *with a warning* --
    /// unlike `width`/`height` under `--tty`, which are dropped silently.
    /// The difference is deliberate: a size has a sensible reading on a
    /// backend that ignores it (the mode wins), whereas naming a DRM
    /// device on a backend with no DRM device at all means the user
    /// believes they are on `--tty` and is not.
    pub gpu: Option<PathBuf>,
    /// Which renderer composites each frame, when the command line says.
    /// `None` (the normal case) means the config file's `[renderer] backend`
    /// decides, and pixman when that is unset too -- so this is
    /// `Option<RendererKind>` rather than a plain `RendererKind` precisely so
    /// that an explicit `--renderer pixman` can *override* a config file
    /// asking for `gles`, the way `--gpu` overrides `[tty] gpu`. Resolved
    /// once, in `compositor::render::resolve`.
    pub renderer: Option<RendererKind>,
    /// `--tty`'s display mode, as `WxH`, when the connector's own preferred
    /// mode is the wrong size. `None` (the normal case) takes the preferred
    /// mode, else the first one listed. `Some` picks the connector mode of
    /// exactly that size, and falls back to the preferred one *with a
    /// warning* when the connector offers no such mode -- the wrong size
    /// beats a black screen. Exists for hosts whose "preferred" size is an
    /// artefact rather than a monitor's: Apple's Virtualization framework
    /// (vfkit, UTM) hands the guest a mode sized from the host window in
    /// backing pixels, so it doubles or halves with the screen the window
    /// happened to open on, while the connector also lists the usual
    /// standard sizes. Meaningless outside `--tty`, where `compositor::run`
    /// ignores it with a warning, for the same reason as `gpu`.
    pub mode: Option<(u16, u16)>,
}

impl Default for CompositorOptions {
    fn default() -> Self {
        Self {
            width: 1600,
            height: 1000,
            outputs: 1,
            socket: None,
            config: None,
            command: Vec::new(),
            nested: false,
            tty: false,
            gpu: None,
            renderer: None,
            mode: None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Error {
    Unknown(String),
    Missing(&'static str),
    Invalid {
        what: &'static str,
        value: String,
    },
    OutOfRange {
        what: &'static str,
        value: String,
        min: i32,
        max: i32,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(what) => write!(f, "unknown argument `{what}` (try --help)"),
            Self::Missing(what) => write!(f, "missing {what} (try --help)"),
            Self::Invalid { what, value } => write!(f, "invalid {what}: `{value}`"),
            Self::OutOfRange {
                what,
                value,
                min,
                max,
            } => write!(f, "invalid {what}: `{value}` (expected {min}-{max})"),
        }
    }
}

impl std::error::Error for Error {}

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Command, Error> {
    let mut args = args.into_iter();
    match args.next().as_deref() {
        None | Some("--help" | "-h" | "help") => Ok(Command::Help),
        Some("--headless") => compositor(args, false, false).map(Command::Compositor),
        Some("--nested") => compositor(args, true, false).map(Command::Compositor),
        Some("--tty") => compositor(args, false, true).map(Command::Compositor),
        Some("msg") => message(args),
        Some(other) => Err(Error::Unknown(other.to_owned())),
    }
}

fn compositor(
    mut args: impl Iterator<Item = String>,
    nested: bool,
    tty: bool,
) -> Result<CompositorOptions, Error> {
    let mut options = CompositorOptions {
        nested,
        tty,
        ..CompositorOptions::default()
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--width" => options.width = dimension("--width", args.next())?,
            "--height" => options.height = dimension("--height", args.next())?,
            "--outputs" => options.outputs = count("--outputs", args.next())?,
            "--socket" => {
                let path = args.next().ok_or(Error::Missing("a path after --socket"))?;
                options.socket = Some(PathBuf::from(path));
            }
            "--config" => {
                let path = args.next().ok_or(Error::Missing("a path after --config"))?;
                options.config = Some(PathBuf::from(path));
            }
            "--gpu" => {
                let path = args.next().ok_or(Error::Missing("a path after --gpu"))?;
                options.gpu = Some(PathBuf::from(path));
            }
            "--renderer" => options.renderer = Some(renderer("--renderer", args.next())?),
            "--mode" => options.mode = Some(mode("--mode", args.next())?),
            "--" => {
                options.command = args.by_ref().collect();
                break;
            }
            other => return Err(Error::Unknown(other.to_owned())),
        }
    }
    Ok(options)
}

/// One of the two renderer names, refused rather than defaulted: a typo'd
/// `--renderer glse` silently compositing on the CPU is exactly the kind of
/// "it ran, but not the way you asked" this project treats as a bug. The
/// config file's copy of the same key is the graceful half (warn, keep the
/// default) -- see `compositor::config` for why the two differ.
fn renderer(what: &'static str, value: Option<String>) -> Result<RendererKind, Error> {
    let value = value.ok_or(Error::Missing("pixman or gles after --renderer"))?;
    RendererKind::parse(&value).ok_or(Error::Invalid { what, value })
}

/// A display mode as `WxH` (`1920x1080`): two positive pixel counts around a
/// lowercase `x`. `u16` because that is what DRM's `Mode::size()` yields,
/// so the comparison in `tty::gpu` is exact rather than converted.
fn mode(what: &'static str, value: Option<String>) -> Result<(u16, u16), Error> {
    let value = value.ok_or(Error::Missing("a WxH size after --mode"))?;
    let invalid = || Error::Invalid {
        what,
        value: value.clone(),
    };
    let (width, height) = value.split_once('x').ok_or_else(invalid)?;
    match (width.parse::<u16>(), height.parse::<u16>()) {
        (Ok(width), Ok(height)) if width > 0 && height > 0 => Ok((width, height)),
        _ => Err(invalid()),
    }
}

fn message(mut args: impl Iterator<Item = String>) -> Result<Command, Error> {
    let verb = args.next().ok_or(Error::Missing("a request"))?;
    let mut out = None;
    let request = match verb.as_str() {
        "version" => Request::Version,
        "outputs" => Request::Outputs,
        "windows" => Request::Windows,
        "action" => Request::Action(action(&mut args)?),
        "screenshot" => {
            let mut output = None;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--output" => output = Some(number::<u64>("--output", args.next())?),
                    "--out" => {
                        out = Some(PathBuf::from(
                            args.next().ok_or(Error::Missing("a path after --out"))?,
                        ))
                    }
                    other => return Err(Error::Unknown(other.to_owned())),
                }
            }
            Request::Screenshot { output }
        }
        "pointer" => pointer(&mut args)?,
        "key" => {
            let combo = args.next().ok_or(Error::Missing("a key combination"))?;
            Request::Key {
                keys: combo.parse().map_err(|_| Error::Invalid {
                    what: "key combination",
                    value: combo.clone(),
                })?,
            }
        }
        "type" => Request::Type {
            text: args.collect::<Vec<_>>().join(" "),
        },
        "wait-idle" => {
            let mut quiet_ms = 200;
            let mut timeout_ms = 5000;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--quiet-ms" => quiet_ms = number("--quiet-ms", args.next())?,
                    "--timeout-ms" => timeout_ms = number("--timeout-ms", args.next())?,
                    other => return Err(Error::Unknown(other.to_owned())),
                }
            }
            Request::WaitIdle {
                quiet_ms,
                timeout_ms,
            }
        }
        other => return Err(Error::Unknown(other.to_owned())),
    };
    Ok(Command::Msg { request, out })
}

/// Parses one action and its arguments (`"focus-column" "left"`, ...) the
/// same way for both `scoot msg action ...` and a config file's `[binds]`
/// values (see `compositor::config::parse_bind`) -- one grammar, one
/// parser, rather than a second copy for the config-file case.
pub(crate) fn action(args: &mut impl Iterator<Item = String>) -> Result<Action, Error> {
    let name = args.next().ok_or(Error::Missing("an action"))?;
    let action = match name.as_str() {
        "focus-column" => Action::FocusColumn {
            direction: horizontal(args)?,
        },
        "move-column" => Action::MoveColumn {
            direction: horizontal(args)?,
        },
        "consume-or-expel" => Action::ConsumeOrExpel {
            direction: horizontal(args)?,
        },
        "focus-window" => Action::FocusWindow {
            direction: vertical(args)?,
        },
        "move-window" => Action::MoveWindow {
            direction: vertical(args)?,
        },
        "focus-workspace" => Action::FocusWorkspace {
            direction: vertical(args)?,
        },
        "move-window-to-workspace" => Action::MoveWindowToWorkspace {
            direction: vertical(args)?,
        },
        "focus-window-id" => Action::FocusWindowId {
            id: number("a window id", args.next())?,
        },
        "focus-workspace-index" => Action::FocusWorkspaceIndex {
            index: number("a workspace index", args.next())?,
        },
        "cycle-column-width" => Action::CycleColumnWidth,
        "close" => Action::CloseFocused,
        "spawn" => {
            let command: Vec<String> = args.collect();
            if command.is_empty() {
                return Err(Error::Missing("a command to spawn"));
            }
            Action::Spawn { command }
        }
        "quit" => Action::Quit,
        other => return Err(Error::Unknown(other.to_owned())),
    };
    Ok(action)
}

fn pointer(args: &mut impl Iterator<Item = String>) -> Result<Request, Error> {
    let kind = args.next().ok_or(Error::Missing("a pointer request"))?;
    let request = match kind.as_str() {
        "move" => Request::PointerMove {
            x: number("x", args.next())?,
            y: number("y", args.next())?,
        },
        "click" => Request::Click {
            x: number("x", args.next())?,
            y: number("y", args.next())?,
            button: args
                .next()
                .map(|b| button(&b))
                .transpose()?
                .unwrap_or_default(),
        },
        "button" => {
            let which = args.next().ok_or(Error::Missing("a button"))?;
            let state = args.next().ok_or(Error::Missing("press or release"))?;
            Request::PointerButton {
                button: button(&which)?,
                pressed: match state.as_str() {
                    "press" => true,
                    "release" => false,
                    _ => {
                        return Err(Error::Invalid {
                            what: "button state",
                            value: state,
                        });
                    }
                },
            }
        }
        "scroll" => Request::Scroll {
            dx: number("dx", args.next())?,
            dy: number("dy", args.next())?,
        },
        other => return Err(Error::Unknown(other.to_owned())),
    };
    Ok(request)
}

fn button(name: &str) -> Result<PointerButton, Error> {
    match name {
        "left" => Ok(PointerButton::Left),
        "right" => Ok(PointerButton::Right),
        "middle" => Ok(PointerButton::Middle),
        other => Err(Error::Invalid {
            what: "button",
            value: other.to_owned(),
        }),
    }
}

fn horizontal(args: &mut impl Iterator<Item = String>) -> Result<Horizontal, Error> {
    match args.next().ok_or(Error::Missing("left or right"))?.as_str() {
        "left" => Ok(Horizontal::Left),
        "right" => Ok(Horizontal::Right),
        other => Err(Error::Invalid {
            what: "direction",
            value: other.to_owned(),
        }),
    }
}

fn vertical(args: &mut impl Iterator<Item = String>) -> Result<Vertical, Error> {
    match args.next().ok_or(Error::Missing("up or down"))?.as_str() {
        "up" => Ok(Vertical::Up),
        "down" => Ok(Vertical::Down),
        other => Err(Error::Invalid {
            what: "direction",
            value: other.to_owned(),
        }),
    }
}

/// A `--width`/`--height` value: a positive pixel count no DRM mode could
/// exceed (see [`MAX_OUTPUT_DIMENSION`]). Refused rather than clamped --
/// every other invalid flag in this file is an `Error::Invalid`, and a
/// typo'd size silently running at a different size would be the worse
/// surprise. Unparsable input keeps the plain `Invalid` shape; a parsed
/// number outside the range echoes the range it was refused for.
fn dimension(what: &'static str, value: Option<String>) -> Result<i32, Error> {
    let raw = value.ok_or(Error::Missing(what))?;
    let size: i32 = raw.parse().map_err(|_| Error::Invalid {
        what,
        value: raw.clone(),
    })?;
    if (1..=MAX_OUTPUT_DIMENSION).contains(&size) {
        Ok(size)
    } else {
        Err(Error::OutOfRange {
            what,
            value: raw,
            min: 1,
            max: MAX_OUTPUT_DIMENSION,
        })
    }
}

/// An `--outputs` value: at least one output, at most [`MAX_OUTPUTS`].
/// Refused rather than clamped, for the same reason [`dimension`] refuses --
/// silently running with a different number of screens than was asked for is
/// the surprise, not the error.
fn count(what: &'static str, value: Option<String>) -> Result<i32, Error> {
    let raw = value.ok_or(Error::Missing(what))?;
    let parsed: i32 = raw.parse().map_err(|_| Error::Invalid {
        what,
        value: raw.clone(),
    })?;
    if (1..=MAX_OUTPUTS).contains(&parsed) {
        Ok(parsed)
    } else {
        Err(Error::OutOfRange {
            what,
            value: raw,
            min: 1,
            max: MAX_OUTPUTS,
        })
    }
}

fn number<T: std::str::FromStr>(what: &'static str, value: Option<String>) -> Result<T, Error> {
    let value = value.ok_or(Error::Missing(what))?;
    value.parse().map_err(|_| Error::Invalid {
        what,
        value: value.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Command, Error> {
        parse(args.iter().map(|a| (*a).to_owned()))
    }

    #[test]
    fn no_arguments_prints_help() {
        assert_eq!(parse_args(&[]), Ok(Command::Help));
    }

    #[test]
    fn compositor_options_have_defaults_and_overrides() {
        let Ok(Command::Compositor(options)) = parse_args(&["--headless"]) else {
            panic!("expected compositor");
        };
        assert_eq!(options, CompositorOptions::default());

        let Ok(Command::Compositor(options)) = parse_args(&[
            "--headless",
            "--width",
            "800",
            "--height",
            "600",
            "--",
            "foot",
            "-e",
            "sh",
        ]) else {
            panic!("expected compositor");
        };
        assert_eq!(options.width, 800);
        assert_eq!(options.height, 600);
        assert_eq!(options.command, vec!["foot", "-e", "sh"]);
        assert!(!options.nested);
    }

    #[test]
    fn width_and_height_refuse_what_no_mode_can_report() {
        // DRM reports each mode axis in a `u16` (`drm_mode_modeinfo`), so
        // nothing real is wider than 65535: anything past it is a typo or a
        // probe, refused the way `--mode` refuses its own bad input rather
        // than silently running at a different size. The refusal echoes the
        // range, so the operator sees the fix, not just the failure.
        for bad in ["0", "-1", "-1600", "65536", "2000000000"] {
            for flag in ["--width", "--height"] {
                let err = parse_args(&["--headless", flag, bad])
                    .expect_err("an absurd size should not parse");
                assert_eq!(
                    err,
                    Error::OutOfRange {
                        what: flag,
                        value: bad.to_owned(),
                        min: 1,
                        max: MAX_OUTPUT_DIMENSION,
                    },
                    "{flag} {bad}"
                );
                assert!(
                    err.to_string().contains("expected 1-65535"),
                    "refusal for {flag} {bad} did not echo the range: {err}"
                );
            }
        }
    }

    #[test]
    fn width_and_height_accept_the_whole_legal_range() {
        // One pixel, today's sizes, 8K/16K, and exactly the DRM maximum.
        for good in ["1", "800", "1920", "7680", "15360", "65535"] {
            let expected: i32 = good.parse().unwrap();
            let Ok(Command::Compositor(options)) =
                parse_args(&["--headless", "--width", good, "--height", good])
            else {
                panic!("expected compositor for {good}");
            };
            assert_eq!((options.width, options.height), (expected, expected));
        }
    }

    #[test]
    fn outputs_defaults_to_one_and_takes_the_whole_legal_range() {
        for mode in ["--headless", "--nested", "--tty"] {
            let Ok(Command::Compositor(options)) = parse_args(&[mode]) else {
                panic!("expected compositor");
            };
            assert_eq!(options.outputs, 1, "{mode}");
        }

        for good in ["1", "2", "3", "8"] {
            let expected: i32 = good.parse().unwrap();
            let Ok(Command::Compositor(options)) = parse_args(&["--headless", "--outputs", good])
            else {
                panic!("expected compositor for {good}");
            };
            assert_eq!(options.outputs, expected);
        }
    }

    #[test]
    fn outputs_refuses_zero_negatives_and_more_screens_than_a_desk_holds() {
        // Refused rather than clamped, and the refusal echoes the range --
        // the same shape `--width`/`--height` have, so a typo is a message
        // rather than a session with a surprising number of screens.
        for bad in ["0", "-1", "9", "100", "2000000000", "two", ""] {
            let err = parse_args(&["--headless", "--outputs", bad])
                .expect_err("an out-of-range output count should not parse");
            match bad.parse::<i32>() {
                Ok(_) => assert_eq!(
                    err,
                    Error::OutOfRange {
                        what: "--outputs",
                        value: bad.to_owned(),
                        min: 1,
                        max: MAX_OUTPUTS,
                    },
                    "{bad}"
                ),
                Err(_) => assert_eq!(
                    err,
                    Error::Invalid {
                        what: "--outputs",
                        value: bad.to_owned(),
                    },
                    "{bad}"
                ),
            }
        }
        assert_eq!(
            parse_args(&["--headless", "--outputs"]),
            Err(Error::Missing("--outputs"))
        );
    }

    #[test]
    fn tty_mode_is_none_by_default_parses_wxh_and_rejects_the_rest() {
        let Ok(Command::Compositor(options)) = parse_args(&["--tty"]) else {
            panic!("expected compositor");
        };
        assert_eq!(options.mode, None);

        let Ok(Command::Compositor(options)) = parse_args(&["--tty", "--mode", "1920x1080"]) else {
            panic!("expected compositor");
        };
        assert!(options.tty);
        assert_eq!(options.mode, Some((1920, 1080)));

        // Anything that is not two positive numbers around an `x`.
        for bad in [
            "1920",
            "1920x",
            "x1080",
            "0x1080",
            "1920x0",
            "1920X1080",
            "wide",
        ] {
            assert_eq!(
                parse_args(&["--tty", "--mode", bad]),
                Err(Error::Invalid {
                    what: "--mode",
                    value: bad.to_owned(),
                }),
                "{bad}"
            );
        }
        assert_eq!(
            parse_args(&["--tty", "--mode"]),
            Err(Error::Missing("a WxH size after --mode"))
        );
    }

    #[test]
    fn config_and_socket_paths_are_none_by_default_and_settable() {
        let Ok(Command::Compositor(options)) = parse_args(&["--headless"]) else {
            panic!("expected compositor");
        };
        assert_eq!(options.config, None);
        assert_eq!(options.socket, None);

        let Ok(Command::Compositor(options)) = parse_args(&[
            "--headless",
            "--config",
            "/etc/scoot/config.toml",
            "--socket",
            "/tmp/scoot.sock",
        ]) else {
            panic!("expected compositor");
        };
        assert_eq!(
            options.config,
            Some(PathBuf::from("/etc/scoot/config.toml"))
        );
        assert_eq!(options.socket, Some(PathBuf::from("/tmp/scoot.sock")));
    }

    #[test]
    fn nested_sets_the_flag_headless_does_not() {
        let Ok(Command::Compositor(options)) = parse_args(&["--nested"]) else {
            panic!("expected compositor");
        };
        assert!(options.nested);

        let Ok(Command::Compositor(options)) = parse_args(&["--headless"]) else {
            panic!("expected compositor");
        };
        assert!(!options.nested);
    }

    #[test]
    fn tty_sets_its_own_flag_and_nothing_else() {
        let Ok(Command::Compositor(options)) = parse_args(&["--tty"]) else {
            panic!("expected compositor");
        };
        assert!(options.tty);
        assert!(!options.nested);

        let Ok(Command::Compositor(options)) = parse_args(&["--headless"]) else {
            panic!("expected compositor");
        };
        assert!(!options.tty);

        let Ok(Command::Compositor(options)) = parse_args(&["--nested"]) else {
            panic!("expected compositor");
        };
        assert!(!options.tty);
    }

    #[test]
    fn gpu_is_none_by_default_and_takes_a_path() {
        let Ok(Command::Compositor(options)) = parse_args(&["--tty"]) else {
            panic!("expected compositor");
        };
        assert_eq!(options.gpu, None);

        let Ok(Command::Compositor(options)) = parse_args(&["--tty", "--gpu", "/dev/dri/card1"])
        else {
            panic!("expected compositor");
        };
        assert_eq!(options.gpu, Some(PathBuf::from("/dev/dri/card1")));
    }

    #[test]
    fn gpu_without_a_path_is_an_error() {
        assert_eq!(
            parse_args(&["--tty", "--gpu"]),
            Err(Error::Missing("a path after --gpu"))
        );
    }

    #[test]
    fn renderer_is_unset_by_default_and_takes_either_name() {
        // Unset, not `Pixman`: the config file only gets a say when the
        // command line has not spoken, so "absent" and "explicitly pixman"
        // cannot be the same value (see `CompositorOptions::renderer`).
        for mode in ["--headless", "--nested", "--tty"] {
            let Ok(Command::Compositor(options)) = parse_args(&[mode]) else {
                panic!("expected compositor");
            };
            assert_eq!(options.renderer, None, "{mode}");
        }

        for (name, expected) in [
            ("pixman", RendererKind::Pixman),
            ("gles", RendererKind::Gles),
        ] {
            let Ok(Command::Compositor(options)) = parse_args(&["--headless", "--renderer", name])
            else {
                panic!("expected compositor for {name}");
            };
            assert_eq!(options.renderer, Some(expected));
        }
    }

    #[test]
    fn an_unknown_renderer_name_is_refused_not_defaulted() {
        // Including the near-misses a typo actually produces, and the empty
        // string: none of them may quietly composite with the other renderer.
        for bad in ["glse", "GLES", "opengl", "gl", "vulkan", "", "pixman "] {
            assert_eq!(
                parse_args(&["--headless", "--renderer", bad]),
                Err(Error::Invalid {
                    what: "--renderer",
                    value: bad.to_owned(),
                }),
                "{bad}"
            );
        }
        assert_eq!(
            parse_args(&["--headless", "--renderer"]),
            Err(Error::Missing("pixman or gles after --renderer"))
        );
    }

    #[test]
    fn the_two_renderer_names_round_trip_through_their_own_spelling() {
        for kind in [RendererKind::Pixman, RendererKind::Gles] {
            assert_eq!(RendererKind::parse(kind.as_str()), Some(kind));
            assert_eq!(kind.to_string(), kind.as_str());
        }
        assert_eq!(RendererKind::default(), RendererKind::Pixman);
    }

    #[test]
    fn actions_take_their_direction() {
        assert_eq!(
            parse_args(&["msg", "action", "focus-column", "left"]),
            Ok(Command::Msg {
                request: Request::Action(Action::FocusColumn {
                    direction: Horizontal::Left
                }),
                out: None,
            })
        );
    }

    #[test]
    fn focus_workspace_index_takes_a_number() {
        assert_eq!(
            parse_args(&["msg", "action", "focus-workspace-index", "2"]),
            Ok(Command::Msg {
                request: Request::Action(Action::FocusWorkspaceIndex { index: 2 }),
                out: None,
            })
        );
        // ...which is a number, not a direction: sharing the
        // `focus-workspace` name would make `up`/`down` and `2` ambiguous.
        assert!(parse_args(&["msg", "action", "focus-workspace-index", "down"]).is_err());
    }

    #[test]
    fn spawn_takes_the_rest_of_the_line() {
        let Ok(Command::Msg { request, .. }) =
            parse_args(&["msg", "action", "spawn", "foot", "-e", "htop"])
        else {
            panic!("expected a message");
        };
        assert_eq!(
            request,
            Request::Action(Action::Spawn {
                command: vec!["foot".into(), "-e".into(), "htop".into()]
            })
        );
    }

    #[test]
    fn typing_keeps_the_words_together() {
        let Ok(Command::Msg { request, .. }) = parse_args(&["msg", "type", "hello", "there"])
        else {
            panic!("expected a message");
        };
        assert_eq!(
            request,
            Request::Type {
                text: "hello there".into()
            }
        );
    }

    #[test]
    fn screenshot_flags_split_between_request_and_output_file() {
        assert_eq!(
            parse_args(&[
                "msg",
                "screenshot",
                "--output",
                "2",
                "--out",
                "/tmp/shot.png"
            ]),
            Ok(Command::Msg {
                request: Request::Screenshot { output: Some(2) },
                out: Some(PathBuf::from("/tmp/shot.png")),
            })
        );
    }

    #[test]
    fn pointer_requests_parse() {
        assert_eq!(
            parse_args(&["msg", "pointer", "click", "10", "20"]),
            Ok(Command::Msg {
                request: Request::Click {
                    x: 10.0,
                    y: 20.0,
                    button: PointerButton::Left
                },
                out: None,
            })
        );
    }

    #[test]
    fn bad_input_explains_itself() {
        assert_eq!(parse_args(&["fly"]), Err(Error::Unknown("fly".into())));
        assert_eq!(parse_args(&["msg"]), Err(Error::Missing("a request")));
        assert_eq!(
            parse_args(&["msg", "action", "focus-column", "sideways"]),
            Err(Error::Invalid {
                what: "direction",
                value: "sideways".into()
            })
        );
        assert_eq!(
            parse_args(&["msg", "pointer", "move", "x", "1"]),
            Err(Error::Invalid {
                what: "x",
                value: "x".into()
            })
        );
    }
}
