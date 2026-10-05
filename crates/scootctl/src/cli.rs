//! Argument parsing for the client: one request per invocation.
//!
//! This module owns the whole client surface -- the request grammar, the
//! `Error` display strings (several are byte-pinned by tests, e.g. the
//! `OutOfRange` echo, so they must not drift), and the help text's prose
//! blocks. Both front-ends (`scootctl` directly, `scoot msg` as its alias)
//! parse through here, and both render their full help through
//! [`crate::help::usage`] -- one table, two renderings (text and JSON) --
//! rather than a second copy of the grammar.
//!
//! The help text is single-sourced by construction: [`REQUESTS_HELP`] and
//! [`ACTIONS_HELP`] are the one copy of the request/action grammar prose,
//! and both this crate's [`usage`] and the compositor binary's `scoot --help`
//! print those same blocks (a containment test here and one in `scoot`'s
//! `cli` tests pin that). The tables in [`crate::help`] are the other half
//! of the source: the examples, exit codes, environment and JSON render
//! from them, and drift tests pin that every row appears in both forms.

use std::fmt;
use std::path::PathBuf;

use scoot_ipc::{Action, EventKind, Horizontal, PointerButton, Request, Vertical};

/// The request grammar both clients print in their `--help`. The single
/// owner: `scoot --help` embeds this same block rather than a second copy.
pub const REQUESTS_HELP: &str = "\
    version | outputs | windows
    action ACTION [ARGUMENT...]
    reload                          re-read the config file and re-apply
                                    what can be re-applied live
    keyboard                        the active keyboard layout's name and
                                    index -- what a layout indicator shows
    output-power ID|all on|off        switch an output's panel off or on --
                                     what an idle daemon drives at idle and
                                     resume (`outputs` reports the state)
    screenshot [--output ID] [--out FILE] [--no-cursor]
                                    the pointer is drawn in unless
                                    --no-cursor
    pointer move X Y | pointer click X Y [left|right|middle|back|forward]
    pointer button left|right|middle|back|forward press|release | pointer scroll DX DY
    key COMBO                       e.g. Return, ctrl+shift+t -- name the key
                                    as it is unmodified plus the modifiers to
                                    hold (shift+1, not exclam)
    type TEXT                       types text, working out each character's
                                    own modifiers from the active layout
    wait-idle [--quiet-ms N] [--timeout-ms N]
    subscribe [EVENT...]          stream events until killed (default: output;
                                    known events: output, keyboard, workspace)
";

/// The action grammar both clients print in their `--help`. Same single-owner
/// arrangement as [`REQUESTS_HELP`]; also the grammar a config file's
/// `[binds]` values use (see [`action`]).
pub const ACTIONS_HELP: &str = "\
    focus-column|move-column|consume-or-expel   left|right
    focus-window|move-window                    up|down
    focus-workspace|move-window-to-workspace    up|down
    focus-window-id ID | focus-workspace-index N [--output ID]
    move-window-to-workspace-index N | focus-output ID
    move-window-to-output ID | focus-output-index N
    move-window-to-output-index N | focus-output-left | focus-output-right
    move-window-to-output-left | move-window-to-output-right
    cycle-column-width | set-column-width N | toggle-fullscreen
    set-fullscreen ID on|off | toggle-maximize | set-maximized ID on|off
    close | spawn COMMAND... | quit
    toggle-floating | set-floating ID on|off | toggle-floating-focus
    move-floating ID X Y | resize-floating ID WIDTH HEIGHT
";

/// The full `--help` text, rendered from the single source in [`crate::help`]:
/// the prose grammar blocks below plus the examples, exit codes, environment
/// and see-also sections the tables generate. A function rather than a
/// `const` so the two forms cannot drift -- the tests pin that every table
/// row appears here and in the JSON.
pub fn usage() -> String {
    crate::help::usage("scootctl", REQUESTS_HELP, ACTIONS_HELP, true)
}

#[derive(Debug, PartialEq)]
pub enum Command {
    Help,
    /// `help [TOPIC]`: one topic's page (`help requests`, `help actions`,
    /// `help exit-codes`, `help environment`).
    Topic(crate::help::Topic),
    /// `help <verb>`: one request verb's own row -- its syntax, one example
    /// and its reply shape. The per-verb spelling (rather than
    /// `<verb> --help`) keeps `type --help` and `action spawn --help`
    /// meaning what they say: `--help` there is text to type or a command
    /// to run, not a plea for help.
    Verb {
        verb: String,
    },
    /// `--help --json` (or `help --json`): the same content as JSON,
    /// versioned (see [`crate::help::SCHEMA_VERSION`]).
    Json,
    /// `scootctl --version`: identify this build without touching the
    /// socket. A first-arg flag like `--help`, answered locally -- never a
    /// request, so it needs no running compositor.
    ///
    /// Deliberately *not* the bare `version` word: that already means the
    /// IPC `Request::Version`, answered by the compositor over the socket.
    /// Giving one spelling two transports (local when idle, remote when a
    /// session happens to be up) would make `scootctl version`'s failure
    /// mode depend on whether a compositor is running; the flag keeps the
    /// two apart.
    Version,
    Msg {
        request: Request,
        out: Option<PathBuf>,
    },
}

/// One parsed client invocation: the request to send, plus where a
/// screenshot's PNG goes (`None` means stdout).
#[derive(Debug, PartialEq)]
pub struct Msg {
    pub request: Request,
    pub out: Option<PathBuf>,
}

#[derive(Debug, PartialEq)]
pub enum Error {
    Unknown(String),
    /// An unknown word with a guess attached: what kind of word it was
    /// expected to be, the closest valid choice, and the help topic that
    /// lists them all. Used at CLI parse sites only -- shared parsers that
    /// also serve the config file (notably [`action`]) keep returning the
    /// bare [`Error::Unknown`] there is no topic for... except the action
    /// name itself, whose topic (`help actions`) is the same grammar in
    /// both places.
    Hinted {
        kind: &'static str,
        what: String,
        suggestion: String,
        topic: &'static str,
    },
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
            Self::Hinted {
                kind,
                what,
                suggestion,
                topic,
            } => write!(
                f,
                "unknown {kind} `{what}` (did you mean `{suggestion}`? see `{topic}`)"
            ),
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

/// An unknown word with a guess attached: [`Error::Hinted`] naming the
/// closest candidate from `candidates`, or the bare [`Error::Unknown`] when
/// nothing is close enough to be a typo rather than a guess (so garbage
/// keeps the old shape). Topics name the `help` page that lists the
/// candidates (`help requests`, `help actions`, ...), without a binary
/// prefix: both front-ends (`scootctl`, `scoot msg`) spell them the same.
fn hinted(kind: &'static str, what: String, candidates: &[&str], topic: &'static str) -> Error {
    match crate::help::suggest(&what, candidates.iter().copied()) {
        Some(suggestion) => Error::Hinted {
            kind,
            what,
            suggestion: suggestion.to_owned(),
            topic,
        },
        None => Error::Unknown(what),
    }
}

/// Every request verb, for `did you mean` over verbs.
fn request_verbs() -> Vec<&'static str> {
    crate::help::REQUESTS
        .iter()
        .map(|request| request.verb)
        .collect()
}

/// Every action name, for `did you mean` over actions.
fn action_names() -> Vec<&'static str> {
    crate::help::ACTIONS
        .iter()
        .map(|action| action.name)
        .collect()
}

/// The `scoot --version` / `scootctl --version` line: the suite's own
/// version plus the IPC protocol number, so a client can check
/// compatibility against a remote compositor before connecting.
///
/// Derived from the same two constants the wire uses --
/// `env!("CARGO_PKG_VERSION")`, which the IPC `version` reply also reads
/// (so the two agree by construction), and [`scoot_ipc::PROTOCOL_VERSION`]
/// -- never a duplicated literal, so the string cannot go stale when
/// either moves. Both binaries print this one helper's return, byte for
/// byte, so a packaging check can compare their outputs directly.
///
/// `env!` here reads *this* crate's version, which is `version.workspace`
/// -- the same workspace version the `scoot` binary's own `env!` reads --
/// so the two binaries' lines agree as long as the workspace version is
/// shared (a test below pins that).
pub fn version_string() -> String {
    format!(
        "scoot {} (ipc protocol {})",
        env!("CARGO_PKG_VERSION"),
        scoot_ipc::PROTOCOL_VERSION
    )
}

/// Parses a full client argv (without the program name): `--help` (or
/// nothing) is help, `help [TOPIC|VERB]` is one page of it, `--help --json`
/// (or `help --json`) is the machine-readable form, `--version` is the local
/// version line, anything else is a request verb.
///
/// Collects into one `Vec` first so the verb stays at the head for
/// [`parse_msg`]'s contract -- a cold path (one process per invocation), so
/// the single small allocation is not load-bearing.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Command, Error> {
    let args: Vec<String> = args.into_iter().collect();
    match args.first().map(String::as_str) {
        None => Ok(Command::Help),
        Some("--help" | "-h" | "help") => help_page(&args[1..]),
        Some("--version") => Ok(Command::Version),
        Some(_) => {
            let Msg { request, out } = message(args.into_iter())?;
            Ok(Command::Msg { request, out })
        }
    }
}

/// `help` / `--help`, optionally followed by one topic name, verb, or
/// `--json`. Anything else is refused with a guess, the way an unknown verb
/// is -- `help` is where a lost agent lands, so it teaches too.
fn help_page(args: &[String]) -> Result<Command, Error> {
    match args {
        [] => Ok(Command::Help),
        [only] if only == "--json" => Ok(Command::Json),
        [only] if only == "help" => Ok(Command::Help),
        [only] => {
            if let Some(topic) = crate::help::Topic::parse(only) {
                return Ok(Command::Topic(topic));
            }
            if crate::help::REQUESTS
                .iter()
                .any(|request| request.verb == only)
            {
                return Ok(Command::Verb { verb: only.clone() });
            }
            let mut candidates: Vec<&str> = crate::help::Topic::names().collect();
            candidates.extend(request_verbs());
            candidates.push("--json");
            Err(hinted("help topic", only.clone(), &candidates, "help"))
        }
        [_, extra, ..] => Err(Error::Unknown(extra.clone())),
    }
}

/// Parses the arguments after the request verb (`scootctl windows ...`, or
/// `scoot msg windows ...` with `msg` already stripped): the verb plus its
/// flags into a [`Msg`].
pub fn parse_msg<I: IntoIterator<Item = String>>(args: I) -> Result<Msg, Error> {
    message(args.into_iter())
}

fn message(mut args: impl Iterator<Item = String>) -> Result<Msg, Error> {
    let verb = args.next().ok_or(Error::Missing("a request"))?;
    let mut out = None;
    let request = match verb.as_str() {
        "version" => Request::Version,
        "outputs" => Request::Outputs,
        "windows" => Request::Windows,
        "reload" => Request::Reload,
        "keyboard" => Request::Keyboard,
        "output-power" => {
            let target = args.next().ok_or(Error::Missing("an output id or `all`"))?;
            let output = match target.as_str() {
                "all" => None,
                other => Some(number::<u64>("an output id", Some(other.to_owned()))?),
            };
            let state = args.next().ok_or(Error::Missing("on or off"))?;
            let powered = match state.as_str() {
                "on" => true,
                "off" => false,
                _ => {
                    return Err(Error::Invalid {
                        what: "power state",
                        value: state,
                    });
                }
            };
            if let Some(extra) = args.next() {
                return Err(Error::Unknown(extra));
            }
            Request::OutputPower { output, powered }
        }
        "action" => Request::Action(action(&mut args)?),
        "screenshot" => {
            let mut output = None;
            let mut cursor = None;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--output" => output = Some(number::<u64>("--output", args.next())?),
                    "--no-cursor" => cursor = Some(false),
                    "--out" => {
                        out = Some(PathBuf::from(
                            args.next().ok_or(Error::Missing("a path after --out"))?,
                        ))
                    }
                    other => {
                        return Err(hinted(
                            "flag",
                            other.to_owned(),
                            &["--output", "--out", "--no-cursor"],
                            "help screenshot",
                        ));
                    }
                }
            }
            Request::Screenshot { output, cursor }
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
                    other => {
                        return Err(hinted(
                            "flag",
                            other.to_owned(),
                            &["--quiet-ms", "--timeout-ms"],
                            "help wait-idle",
                        ));
                    }
                }
            }
            Request::WaitIdle {
                quiet_ms,
                timeout_ms,
            }
        }
        "subscribe" => {
            let mut events = Vec::new();
            for name in args {
                match name.as_str() {
                    "output" => events.push(EventKind::Output),
                    "keyboard" => events.push(EventKind::Keyboard),
                    "workspace" => events.push(EventKind::Workspace),
                    other => {
                        return Err(
                            match crate::help::suggest(other, ["output", "keyboard", "workspace"]) {
                                Some(suggestion) => Error::Hinted {
                                    kind: "event",
                                    what: other.to_owned(),
                                    suggestion: suggestion.to_owned(),
                                    topic: "help subscribe",
                                },
                                None => Error::Unknown(format!(
                                    "event {other} (known events: output, keyboard, workspace)"
                                )),
                            },
                        );
                    }
                }
            }
            if events.is_empty() {
                events.push(EventKind::Output);
            }
            Request::Subscribe { events }
        }
        other => {
            return Err(hinted(
                "request",
                other.to_owned(),
                &request_verbs(),
                "help requests",
            ));
        }
    };
    Ok(Msg { request, out })
}

/// Parses one action and its arguments (`"focus-column" "left"`, ...) the
/// same way for both `scootctl action ...` (and its `scoot msg action ...`
/// alias) and a config file's `[binds]` values (see
/// `scoot::compositor::config::parse_bind`) -- one grammar, one parser,
/// rather than a second copy for the config-file case.
pub fn action(args: &mut impl Iterator<Item = String>) -> Result<Action, Error> {
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
        "focus-workspace-index" => {
            let index = number("a workspace index", args.next())?;
            // The only flag an action takes: without it this names the
            // focused output's list, with it one specific output's. Anything
            // else trailing is a loud refusal, the way `screenshot`'s flag
            // loop answers an unknown flag (binds and autostart reject
            // trailing text either way).
            let mut output: Option<u64> = None;
            let rest: Vec<String> = args.by_ref().collect();
            let mut rest = rest.into_iter();
            while let Some(flag) = rest.next() {
                match flag.as_str() {
                    "--output" => output = Some(number("an output id", rest.next())?),
                    other => {
                        return Err(hinted(
                            "flag",
                            other.to_owned(),
                            &["--output"],
                            "help actions",
                        ));
                    }
                }
            }
            match output {
                Some(output) => Action::FocusOutputWorkspaceIndex { output, index },
                None => Action::FocusWorkspaceIndex { index },
            }
        }
        "move-window-to-workspace-index" => Action::MoveWindowToWorkspaceIndex {
            index: number("a workspace index", args.next())?,
        },
        "focus-output" => Action::FocusOutput {
            output: number("an output id", args.next())?,
        },
        "move-window-to-output" => Action::MoveFocusedWindowToOutput {
            output: number("an output id", args.next())?,
        },
        "focus-output-index" => Action::FocusOutputIndex {
            index: number("an output index", args.next())?,
        },
        "move-window-to-output-index" => Action::MoveFocusedWindowToOutputIndex {
            index: number("an output index", args.next())?,
        },
        // The stepping halves: one verb per side (niri's
        // `focus-monitor-left` shape), so `focus-output` keeps meaning
        // exactly one thing -- an output id -- and neither verb overloads
        // the other's argument.
        "focus-output-left" => Action::FocusOutputDirection {
            direction: Horizontal::Left,
        },
        "focus-output-right" => Action::FocusOutputDirection {
            direction: Horizontal::Right,
        },
        "move-window-to-output-left" => Action::MoveWindowToOutputDirection {
            direction: Horizontal::Left,
        },
        "move-window-to-output-right" => Action::MoveWindowToOutputDirection {
            direction: Horizontal::Right,
        },
        "cycle-column-width" => Action::CycleColumnWidth,
        "set-column-width" => Action::SetColumnWidth {
            index: number("a column width index", args.next())?,
        },
        "toggle-fullscreen" => Action::ToggleFullscreen,
        "set-fullscreen" => Action::SetFullscreen {
            id: number("a window id", args.next())?,
            fullscreen: match args.next().ok_or(Error::Missing("on or off"))?.as_str() {
                "on" => true,
                "off" => false,
                other => {
                    return Err(Error::Invalid {
                        what: "fullscreen state",
                        value: other.to_owned(),
                    });
                }
            },
        },
        "toggle-maximize" => Action::ToggleMaximize,
        "set-maximized" => Action::SetMaximized {
            id: number("a window id", args.next())?,
            maximized: match args.next().ok_or(Error::Missing("on or off"))?.as_str() {
                "on" => true,
                "off" => false,
                other => {
                    return Err(Error::Invalid {
                        what: "maximized state",
                        value: other.to_owned(),
                    });
                }
            },
        },
        "toggle-floating" => Action::ToggleFloating,
        "set-floating" => Action::SetFloating {
            id: number("a window id", args.next())?,
            floating: match args.next().ok_or(Error::Missing("on or off"))?.as_str() {
                "on" => true,
                "off" => false,
                other => {
                    return Err(Error::Invalid {
                        what: "floating state",
                        value: other.to_owned(),
                    });
                }
            },
        },
        "toggle-floating-focus" => Action::ToggleFloatingFocus,
        "move-floating" => Action::MoveFloating {
            id: number("a window id", args.next())?,
            x: number("an x coordinate", args.next())?,
            y: number("a y coordinate", args.next())?,
        },
        "resize-floating" => Action::ResizeFloating {
            id: number("a window id", args.next())?,
            width: number("a width", args.next())?,
            height: number("a height", args.next())?,
        },
        "close" => Action::CloseFocused,
        "spawn" => {
            let command: Vec<String> = args.collect();
            if command.is_empty() {
                return Err(Error::Missing("a command to spawn"));
            }
            Action::Spawn { command }
        }
        "quit" => Action::Quit,
        other => {
            return Err(hinted(
                "action",
                other.to_owned(),
                &action_names(),
                "help actions",
            ));
        }
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
        other => {
            return Err(hinted(
                "pointer request",
                other.to_owned(),
                &["move", "click", "button", "scroll"],
                "help pointer",
            ));
        }
    };
    Ok(request)
}

fn button(name: &str) -> Result<PointerButton, Error> {
    match name {
        "left" => Ok(PointerButton::Left),
        "right" => Ok(PointerButton::Right),
        "middle" => Ok(PointerButton::Middle),
        "back" => Ok(PointerButton::Back),
        "forward" => Ok(PointerButton::Forward),
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

    fn parse_msg_args(args: &[&str]) -> Result<Msg, Error> {
        parse_msg(args.iter().map(|a| (*a).to_owned()))
    }

    fn parse_args(args: &[&str]) -> Result<Command, Error> {
        parse(args.iter().map(|a| (*a).to_owned()))
    }

    #[test]
    fn no_arguments_and_help_flags_print_help() {
        assert_eq!(parse_args(&[]), Ok(Command::Help));
        for flag in ["--help", "-h", "help"] {
            assert_eq!(parse_args(&[flag]), Ok(Command::Help), "{flag}");
        }
    }

    #[test]
    fn usage_prints_the_shared_grammar_blocks() {
        // The single-ownership pin: the full help text embeds the same two
        // blocks `scoot --help` embeds (pinned from that side in scoot's
        // `cli` tests), so editing one copy without the other fails here.
        let text = usage();
        assert!(text.contains(REQUESTS_HELP), "usage lost REQUESTS_HELP");
        assert!(text.contains(ACTIONS_HELP), "usage lost ACTIONS_HELP");
    }

    #[test]
    fn help_pages_route_to_their_topic_verb_or_json() {
        assert_eq!(parse_args(&[]), Ok(Command::Help));
        assert_eq!(
            parse_args(&["help", "requests"]),
            Ok(Command::Topic(crate::help::Topic::Requests))
        );
        assert_eq!(
            parse_args(&["help", "actions"]),
            Ok(Command::Topic(crate::help::Topic::Actions))
        );
        assert_eq!(
            parse_args(&["help", "exit-codes"]),
            Ok(Command::Topic(crate::help::Topic::ExitCodes))
        );
        assert_eq!(
            parse_args(&["help", "environment"]),
            Ok(Command::Topic(crate::help::Topic::Environment))
        );
        assert_eq!(
            parse_args(&["help", "screenshot"]),
            Ok(Command::Verb {
                verb: "screenshot".into()
            })
        );
        assert_eq!(parse_args(&["help", "--json"]), Ok(Command::Json));
        assert_eq!(parse_args(&["--help", "--json"]), Ok(Command::Json));
        assert_eq!(parse_args(&["help", "help"]), Ok(Command::Help));
        assert_eq!(
            parse_args(&["--help", "actions"]),
            Ok(Command::Topic(crate::help::Topic::Actions))
        );
    }

    #[test]
    fn an_unknown_help_topic_teaches() {
        // Close enough to guess: the nearest topic, named with where to
        // read. Garbage stays a bare refusal.
        assert_eq!(
            parse_args(&["help", "request"]),
            Err(Error::Hinted {
                kind: "help topic",
                what: "request".into(),
                suggestion: "requests".into(),
                topic: "help",
            })
        );
        assert!(matches!(
            parse_args(&["help", "xyzzy"]),
            Err(Error::Unknown(_))
        ));
        assert!(matches!(
            parse_args(&["help", "requests", "extra"]),
            Err(Error::Unknown(_))
        ));
    }

    #[test]
    fn usage_errors_name_the_nearest_valid_choice() {
        // The agent test: a typo'd verb is answered with the verb it meant
        // and the topic that lists them, not just "unknown argument".
        assert_eq!(
            parse_args(&["windwos"]),
            Err(Error::Hinted {
                kind: "request",
                what: "windwos".into(),
                suggestion: "windows".into(),
                topic: "help requests",
            })
        );
        assert_eq!(
            parse_args(&["action", "togle-fullscreen"]),
            Err(Error::Hinted {
                kind: "action",
                what: "togle-fullscreen".into(),
                suggestion: "toggle-fullscreen".into(),
                topic: "help actions",
            })
        );
        assert_eq!(
            parse_args(&["screenshot", "--ouput", "1"]),
            Err(Error::Hinted {
                kind: "flag",
                what: "--ouput".into(),
                suggestion: "--output".into(),
                topic: "help screenshot",
            })
        );
        let error = parse_args(&["windwos"]).expect_err("a typo is refused");
        assert_eq!(
            error.to_string(),
            "unknown request `windwos` (did you mean `windows`? see `help requests`)"
        );
    }

    #[test]
    fn version_flag_parses_to_its_own_command() {
        // A first-arg flag like `--help`, not a request: it is answered
        // locally, with no socket and no running compositor.
        assert_eq!(parse_args(&["--version"]), Ok(Command::Version));
    }

    #[test]
    fn version_flag_is_first_arg_like_help() {
        // Trailing arguments are ignored, the way `--help foo` still
        // prints help; a flag in front is not `--version` at all, the way
        // `--headless --help` is not help either.
        assert_eq!(
            parse_args(&["--version", "--headless", "foo"]),
            Ok(Command::Version)
        );
        assert_eq!(
            parse_args(&["--headless", "--version"]),
            Err(Error::Unknown("--headless".into()))
        );
    }

    #[test]
    fn a_bare_version_word_stays_the_ipc_request() {
        // The spelling decision, pinned: `version` (bare) is the remote
        // request answered by the compositor over the socket --
        // `scootctl version` with no session running must fail, not answer
        // locally -- while `--version` is the local flag. One spelling, one
        // transport each.
        assert_eq!(
            parse_args(&["version"]),
            Ok(Command::Msg {
                request: Request::Version,
                out: None,
            })
        );
        assert_eq!(parse_args(&["--version"]), Ok(Command::Version));
    }

    #[test]
    fn version_string_tracks_both_constants() {
        // The no-drift pin: the expected line is recomputed from the same
        // two constants the helper derives from, so moving either constant
        // without the string fails here. A hardcoded `"4"` surviving a
        // `PROTOCOL_VERSION` 4 -> 5 is exactly what this catches.
        assert_eq!(
            version_string(),
            format!(
                "scoot {} (ipc protocol {})",
                env!("CARGO_PKG_VERSION"),
                scoot_ipc::PROTOCOL_VERSION
            )
        );
        // And the shape the ticket fixes: `scoot 0.1.0 (ipc protocol 7)`,
        // one line, no trailing newline (`print_line` adds it).
        assert!(version_string().starts_with("scoot "));
        assert!(!version_string().ends_with('\n'));
    }

    #[test]
    fn usage_names_version_on_its_own_line() {
        // The `--help` surface for the new flag: its own usage line, so a
        // user reading `--help` can discover it without knowing the ticket.
        assert!(
            usage()
                .lines()
                .any(|line| line.trim() == "scootctl --version"),
            "--help hides the version flag"
        );
    }

    #[test]
    fn a_bare_request_parses() {
        assert_eq!(
            parse_args(&["windows"]),
            Ok(Command::Msg {
                request: Request::Windows,
                out: None,
            })
        );
        assert_eq!(
            parse_args(&["reload"]),
            Ok(Command::Msg {
                request: Request::Reload,
                out: None,
            })
        );
        assert_eq!(
            parse_args(&["keyboard"]),
            Ok(Command::Msg {
                request: Request::Keyboard,
                out: None,
            })
        );
        assert_eq!(
            parse_args(&["version"]),
            Ok(Command::Msg {
                request: Request::Version,
                out: None,
            })
        );
    }

    #[test]
    fn output_power_takes_an_id_or_all_and_on_or_off() {
        assert_eq!(
            parse_msg_args(&["output-power", "2", "off"]),
            Ok(Msg {
                request: Request::OutputPower {
                    output: Some(2),
                    powered: false,
                },
                out: None,
            })
        );
        assert_eq!(
            parse_msg_args(&["output-power", "all", "on"]),
            Ok(Msg {
                request: Request::OutputPower {
                    output: None,
                    powered: true,
                },
                out: None,
            })
        );
        // An id is a number, the state is on/off, nothing trails.
        assert!(parse_msg_args(&["output-power", "down", "off"]).is_err());
        assert!(parse_msg_args(&["output-power", "2"]).is_err());
        assert!(parse_msg_args(&["output-power", "2", "yes"]).is_err());
        assert!(parse_msg_args(&["output-power", "all", "off", "extra"]).is_err());
        assert!(parse_msg_args(&["output-power"]).is_err());
    }

    #[test]
    fn usage_names_output_power_on_its_own_line() {
        assert!(
            usage()
                .lines()
                .any(|line| line.trim().starts_with("output-power ID|all")),
            "--help hides the output-power verb"
        );
    }

    #[test]
    fn usage_names_keyboard_on_its_own_line() {
        assert!(
            usage()
                .lines()
                .any(|line| line.trim().starts_with("keyboard")),
            "--help hides the keyboard verb"
        );
    }

    #[test]
    fn subscribe_defaults_to_output_and_names_what_it_takes() {
        assert_eq!(
            parse_args(&["subscribe"]),
            Ok(Command::Msg {
                request: Request::Subscribe {
                    events: vec![EventKind::Output]
                },
                out: None,
            })
        );
        assert_eq!(
            parse_msg_args(&["subscribe", "output"]),
            Ok(Msg {
                request: Request::Subscribe {
                    events: vec![EventKind::Output]
                },
                out: None,
            })
        );
        assert_eq!(
            parse_msg_args(&["subscribe", "keyboard"]),
            Ok(Msg {
                request: Request::Subscribe {
                    events: vec![EventKind::Keyboard]
                },
                out: None,
            })
        );
        assert_eq!(
            parse_msg_args(&["subscribe", "output", "keyboard"]),
            Ok(Msg {
                request: Request::Subscribe {
                    events: vec![EventKind::Output, EventKind::Keyboard]
                },
                out: None,
            })
        );
        assert_eq!(
            parse_msg_args(&["subscribe", "workspace"]),
            Ok(Msg {
                request: Request::Subscribe {
                    events: vec![EventKind::Workspace]
                },
                out: None,
            })
        );
        assert_eq!(
            parse_msg_args(&["subscribe", "output", "workspace"]),
            Ok(Msg {
                request: Request::Subscribe {
                    events: vec![EventKind::Output, EventKind::Workspace]
                },
                out: None,
            })
        );
        assert!(
            matches!(
                parse_args(&["subscribe", "hypothetical_future_kind"]),
                Err(Error::Unknown(_))
            ),
            "an unknown event kind is a loud refusal naming the rule, not a guess"
        );
    }

    #[test]
    fn usage_names_subscribe_on_its_own_line() {
        assert!(
            usage()
                .lines()
                .any(|line| line.trim().starts_with("subscribe [EVENT...]")),
            "--help hides the subscribe verb"
        );
    }

    #[test]
    fn actions_take_their_direction() {
        assert_eq!(
            parse_msg_args(&["action", "focus-column", "left"]),
            Ok(Msg {
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
            parse_msg_args(&["action", "focus-workspace-index", "2"]),
            Ok(Msg {
                request: Request::Action(Action::FocusWorkspaceIndex { index: 2 }),
                out: None,
            })
        );
        // ...which is a number, not a direction: sharing the
        // `focus-workspace` name would make `up`/`down` and `2` ambiguous.
        assert!(parse_msg_args(&["action", "focus-workspace-index", "down"]).is_err());
    }

    #[test]
    fn focus_workspace_index_names_an_output_with_the_flag() {
        // With `--output` this is the targeted switch: one specific
        // output's list, and focus follows it there.
        assert_eq!(
            parse_msg_args(&["action", "focus-workspace-index", "2", "--output", "5"]),
            Ok(Msg {
                request: Request::Action(Action::FocusOutputWorkspaceIndex {
                    output: 5,
                    index: 2
                }),
                out: None,
            })
        );
        // A flag with no id, a non-numeric id, and anything but the flag
        // are all loud refusals rather than a silent focus-output switch.
        assert!(parse_msg_args(&["action", "focus-workspace-index", "2", "--output"]).is_err());
        assert!(
            parse_msg_args(&["action", "focus-workspace-index", "2", "--output", "x"]).is_err()
        );
        assert!(parse_msg_args(&["action", "focus-workspace-index", "2", "5"]).is_err());
        assert!(
            parse_msg_args(&["action", "focus-workspace-index", "2", "--output", "5", "x"])
                .is_err()
        );
    }

    #[test]
    fn move_window_to_workspace_index_takes_a_number() {
        assert_eq!(
            parse_msg_args(&["action", "move-window-to-workspace-index", "3"]),
            Ok(Msg {
                request: Request::Action(Action::MoveWindowToWorkspaceIndex { index: 3 }),
                out: None,
            })
        );
        // A number, not a direction -- mirroring `focus-workspace-index`:
        // sharing the `move-window-to-workspace` name would make `up`/`down`
        // and `3` ambiguous.
        assert!(parse_msg_args(&["action", "move-window-to-workspace-index", "down"]).is_err());
        assert!(parse_msg_args(&["action", "move-window-to-workspace-index"]).is_err());
    }

    #[test]
    fn set_column_width_takes_a_number() {
        assert_eq!(
            parse_msg_args(&["action", "set-column-width", "2"]),
            Ok(Msg {
                request: Request::Action(Action::SetColumnWidth { index: 2 }),
                out: None,
            })
        );
        // A number, not a direction -- mirroring the workspace-index pair:
        // sharing the `cycle-column-width` name would make `2` ambiguous
        // against a bare cycle.
        assert!(parse_msg_args(&["action", "set-column-width", "down"]).is_err());
        assert!(parse_msg_args(&["action", "set-column-width"]).is_err());
    }

    #[test]
    fn toggle_fullscreen_takes_no_argument() {
        assert_eq!(
            parse_msg_args(&["action", "toggle-fullscreen"]),
            Ok(Msg {
                request: Request::Action(Action::ToggleFullscreen),
                out: None,
            })
        );
    }

    #[test]
    fn set_fullscreen_takes_a_window_id_and_on_or_off() {
        for (word, fullscreen) in [("on", true), ("off", false)] {
            assert_eq!(
                parse_msg_args(&["action", "set-fullscreen", "7", word]),
                Ok(Msg {
                    request: Request::Action(Action::SetFullscreen { id: 7, fullscreen }),
                    out: None,
                })
            );
        }
        assert!(parse_msg_args(&["action", "set-fullscreen", "7"]).is_err());
        assert!(parse_msg_args(&["action", "set-fullscreen", "7", "yes"]).is_err());
        assert!(parse_msg_args(&["action", "set-fullscreen", "on"]).is_err());
        assert!(parse_msg_args(&["action", "set-fullscreen"]).is_err());
    }

    #[test]
    fn toggle_maximize_takes_no_argument() {
        assert_eq!(
            parse_msg_args(&["action", "toggle-maximize"]),
            Ok(Msg {
                request: Request::Action(Action::ToggleMaximize),
                out: None,
            })
        );
    }

    #[test]
    fn set_maximized_takes_a_window_id_and_on_or_off() {
        for (word, maximized) in [("on", true), ("off", false)] {
            assert_eq!(
                parse_msg_args(&["action", "set-maximized", "7", word]),
                Ok(Msg {
                    request: Request::Action(Action::SetMaximized { id: 7, maximized }),
                    out: None,
                })
            );
        }
        assert!(parse_msg_args(&["action", "set-maximized", "7"]).is_err());
        assert!(parse_msg_args(&["action", "set-maximized", "7", "yes"]).is_err());
        assert!(parse_msg_args(&["action", "set-maximized", "on"]).is_err());
        assert!(parse_msg_args(&["action", "set-maximized"]).is_err());
    }

    #[test]
    fn the_floating_toggles_take_no_argument() {
        for (word, action) in [
            ("toggle-floating", Action::ToggleFloating),
            ("toggle-floating-focus", Action::ToggleFloatingFocus),
        ] {
            assert_eq!(
                parse_msg_args(&["action", word]),
                Ok(Msg {
                    request: Request::Action(action),
                    out: None,
                })
            );
        }
    }

    #[test]
    fn the_floating_geometry_actions_take_an_id_and_two_numbers() {
        assert_eq!(
            parse_msg_args(&["action", "move-floating", "7", "-30", "40"]),
            Ok(Msg {
                request: Request::Action(Action::MoveFloating {
                    id: 7,
                    x: -30,
                    y: 40
                }),
                out: None,
            })
        );
        assert_eq!(
            parse_msg_args(&["action", "resize-floating", "7", "640", "480"]),
            Ok(Msg {
                request: Request::Action(Action::ResizeFloating {
                    id: 7,
                    width: 640,
                    height: 480
                }),
                out: None,
            })
        );
        for bad in [
            &["action", "move-floating", "7", "10"][..],
            &["action", "move-floating", "7", "x", "10"],
            &["action", "resize-floating", "7", "-1", "10"],
            &["action", "resize-floating", "7", "640"],
            &["action", "resize-floating"],
        ] {
            assert!(parse_msg_args(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn set_floating_takes_a_window_id_and_on_or_off() {
        for (word, floating) in [("on", true), ("off", false)] {
            assert_eq!(
                parse_msg_args(&["action", "set-floating", "7", word]),
                Ok(Msg {
                    request: Request::Action(Action::SetFloating { id: 7, floating }),
                    out: None,
                })
            );
        }
        assert!(parse_msg_args(&["action", "set-floating", "7"]).is_err());
        assert!(parse_msg_args(&["action", "set-floating", "7", "yes"]).is_err());
        assert!(parse_msg_args(&["action", "set-floating", "on"]).is_err());
        assert!(parse_msg_args(&["action", "set-floating"]).is_err());
    }

    #[test]
    fn the_output_actions_take_an_output_id() {
        // Output ids, not workspace positions: ids are stable for the
        // session (`scootctl outputs` reports them), while a workspace index
        // only means anything within one output's list.
        assert_eq!(
            parse_msg_args(&["action", "focus-output", "2"]),
            Ok(Msg {
                request: Request::Action(Action::FocusOutput { output: 2 }),
                out: None,
            })
        );
        assert_eq!(
            parse_msg_args(&["action", "move-window-to-output", "2"]),
            Ok(Msg {
                request: Request::Action(Action::MoveFocusedWindowToOutput { output: 2 }),
                out: None,
            })
        );
        assert!(parse_msg_args(&["action", "focus-output", "down"]).is_err());
        assert!(parse_msg_args(&["action", "move-window-to-output"]).is_err());
    }

    #[test]
    fn the_positional_output_actions_take_an_output_index() {
        // Positions, not ids: 0-based into the output list in creation
        // order, so the second screen is 1 whatever id it carries.
        assert_eq!(
            parse_msg_args(&["action", "focus-output-index", "1"]),
            Ok(Msg {
                request: Request::Action(Action::FocusOutputIndex { index: 1 }),
                out: None,
            })
        );
        assert_eq!(
            parse_msg_args(&["action", "move-window-to-output-index", "1"]),
            Ok(Msg {
                request: Request::Action(Action::MoveFocusedWindowToOutputIndex { index: 1 }),
                out: None,
            })
        );
        assert!(parse_msg_args(&["action", "focus-output-index", "down"]).is_err());
        assert!(parse_msg_args(&["action", "move-window-to-output-index"]).is_err());
    }

    #[test]
    fn the_relative_output_actions_take_no_argument() {
        // One verb per side, no argument: stepping is relative to the
        // focused output, so there is nothing to name -- and `focus-output`
        // keeps taking exactly an output id, never a direction.
        for (verb, action) in [
            (
                "focus-output-left",
                Action::FocusOutputDirection {
                    direction: Horizontal::Left,
                },
            ),
            (
                "focus-output-right",
                Action::FocusOutputDirection {
                    direction: Horizontal::Right,
                },
            ),
            (
                "move-window-to-output-left",
                Action::MoveWindowToOutputDirection {
                    direction: Horizontal::Left,
                },
            ),
            (
                "move-window-to-output-right",
                Action::MoveWindowToOutputDirection {
                    direction: Horizontal::Right,
                },
            ),
        ] {
            assert_eq!(
                parse_msg_args(&["action", verb]),
                Ok(Msg {
                    request: Request::Action(action),
                    out: None,
                }),
                "{verb}"
            );
        }
        // ...and neither old verb learned a direction: an id is still an
        // id.
        assert!(parse_msg_args(&["action", "focus-output", "left"]).is_err());
        assert!(parse_msg_args(&["action", "move-window-to-output", "right"]).is_err());
    }

    #[test]
    fn spawn_takes_the_rest_of_the_line() {
        let Ok(Msg { request, .. }) = parse_msg_args(&["action", "spawn", "foot", "-e", "htop"])
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
        let Ok(Msg { request, .. }) = parse_msg_args(&["type", "hello", "there"]) else {
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
            parse_msg_args(&["screenshot", "--output", "2", "--out", "/tmp/shot.png"]),
            Ok(Msg {
                request: Request::Screenshot {
                    output: Some(2),
                    cursor: None,
                },
                out: Some(PathBuf::from("/tmp/shot.png")),
            })
        );
    }

    #[test]
    fn screenshot_no_cursor_asks_for_the_pointer_left_out() {
        // Only the opt-out is a flag: leaving it off sends no field at all,
        // which the server reads as its documented default (drawn in).
        assert_eq!(
            parse_msg_args(&["screenshot", "--no-cursor", "--out", "/tmp/shot.png"]),
            Ok(Msg {
                request: Request::Screenshot {
                    output: None,
                    cursor: Some(false),
                },
                out: Some(PathBuf::from("/tmp/shot.png")),
            })
        );
        assert_eq!(
            parse_msg_args(&["screenshot"]),
            Ok(Msg {
                request: Request::Screenshot {
                    output: None,
                    cursor: None,
                },
                out: None,
            })
        );
    }

    #[test]
    fn pointer_requests_parse() {
        assert_eq!(
            parse_msg_args(&["pointer", "click", "10", "20"]),
            Ok(Msg {
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
        assert_eq!(parse_msg_args(&[]), Err(Error::Missing("a request")));
        assert_eq!(
            parse_msg_args(&["action", "focus-column", "sideways"]),
            Err(Error::Invalid {
                what: "direction",
                value: "sideways".into()
            })
        );
        assert_eq!(
            parse_msg_args(&["pointer", "move", "x", "1"]),
            Err(Error::Invalid {
                what: "x",
                value: "x".into()
            })
        );
    }
}
