//! Argument parsing. Deliberately small: flexwm's surface is one compositor to
//! start and a handful of requests to send.

use std::fmt;
use std::path::PathBuf;

use flexwm_ipc::{Action, Horizontal, PointerButton, Request, Vertical};

pub const USAGE: &str = "\
flexwm -- a scrolling-tiling Wayland compositor

USAGE:
    flexwm --headless [--width W] [--height H] [--socket PATH] [--config PATH] [-- COMMAND...]
    flexwm --nested [--width W] [--height H] [--socket PATH] [--config PATH] [-- COMMAND...]
    flexwm --tty [--gpu PATH] [--mode WxH] [--socket PATH] [--config PATH] [-- COMMAND...]
    flexwm msg REQUEST
    flexwm --help

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
    /// The requested size. Under `--nested`, the host's first configure can
    /// override this; under `--headless` it's authoritative, there being no
    /// host to negotiate with.
    pub width: i32,
    pub height: i32,
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
    /// wrong can name the right device instead. Meaningless outside
    /// `--tty`, where `compositor::run` ignores it *with a warning* --
    /// unlike `width`/`height` under `--tty`, which are dropped silently.
    /// The difference is deliberate: a size has a sensible reading on a
    /// backend that ignores it (the mode wins), whereas naming a DRM
    /// device on a backend with no DRM device at all means the user
    /// believes they are on `--tty` and is not.
    pub gpu: Option<PathBuf>,
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
            socket: None,
            config: None,
            command: Vec::new(),
            nested: false,
            tty: false,
            gpu: None,
            mode: None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Error {
    Unknown(String),
    Missing(&'static str),
    Invalid { what: &'static str, value: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(what) => write!(f, "unknown argument `{what}` (try --help)"),
            Self::Missing(what) => write!(f, "missing {what} (try --help)"),
            Self::Invalid { what, value } => write!(f, "invalid {what}: `{value}`"),
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
            "--width" => options.width = number("--width", args.next())?,
            "--height" => options.height = number("--height", args.next())?,
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
/// same way for both `flexwm msg action ...` and a config file's `[binds]`
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
            "/etc/flexwm/config.toml",
            "--socket",
            "/tmp/flexwm.sock",
        ]) else {
            panic!("expected compositor");
        };
        assert_eq!(
            options.config,
            Some(PathBuf::from("/etc/flexwm/config.toml"))
        );
        assert_eq!(options.socket, Some(PathBuf::from("/tmp/flexwm.sock")));
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
