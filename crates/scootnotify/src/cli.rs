//! `scootnotify daemon [--check]` and `scootnotify msg …`: the command line.
//!
//! Two subcommands: `daemon` owns `org.freedesktop.Notifications` until
//! killed, and `msg` asks the running daemon over its control socket
//! (`list`, `dismiss`, `invoke`, `reload`, `subscribe`, `wait`,
//! `version`). Exit statuses: 0 done, 1 failed, 2 misused, 3 the name is
//! owned (the unit's `RestartPreventExitStatus=3` turns that into a
//! stopped unit, never a restart loop).

use crate::control::protocol::EventKind;

/// What `daemon` and `msg` run with: everything the usage below promises.
#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    /// Which subcommand: `daemon` serves, `msg` asks.
    pub command: Command,
    /// `--check`: validate the config and exit, without a bus.
    pub check: bool,
    /// `--config PATH`: read this file instead of the default.
    pub config: Option<String>,
    /// `--font PATH`: draw with this font file instead of the first
    /// well-known one (the look's `font` key names the family; this
    /// names the file, as the panel's does).
    pub font: Option<String>,
    /// `--help` / `--version`: handled before anything else.
    pub help: bool,
    pub version: bool,
}

/// The subcommand.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Daemon,
    Msg(Msg),
}

/// One `msg` command.
#[derive(Debug, PartialEq, Eq)]
pub enum Msg {
    /// The live notifications: human lines, or the raw JSON with `--json`.
    List { json: bool },
    /// Close one card, or every live card with `--all`, as a click would.
    Dismiss { id: Option<u32>, all: bool },
    /// Run a card's action as a click would (`default` when absent).
    Invoke { id: u32, action: String },
    /// Re-read the config file and the look layer (`--look`: only the
    /// look layer, never touching the user file).
    Reload { look: bool },
    /// Stay connected and print events of these kinds (all of them when
    /// none are named).
    Subscribe { events: Vec<EventKind> },
    /// Block until a notification matches, or the timeout lapses.
    Wait {
        app: Option<String>,
        summary: Option<String>,
        timeout_secs: u64,
    },
    /// The daemon's version and protocol.
    Version,
}

/// `msg wait`'s timeout when `--timeout` is absent: long enough for a
/// download to finish, short enough that a forgotten wait does not linger.
pub const DEFAULT_WAIT_TIMEOUT_SECS: u64 = 30;

/// The usage both `--help` and the misuse refusal print.
pub const HELP: &str = "scootnotify: the notification daemon\n\
    \n\
    Usage:\n\
    \tscootnotify daemon [--check] [--config PATH] [--font PATH]\n\
    \tscootnotify msg list [--json]\n\
    \tscootnotify msg dismiss [ID | --all]\n\
    \tscootnotify msg invoke ID [ACTION]\n\
    \tscootnotify msg reload [--look]\n\
    \tscootnotify msg subscribe [notify] [close] [invoke]\n\
    \tscootnotify msg wait [--app APP] [--summary-contains TEXT] [--timeout SECS]\n\
    \tscootnotify msg version\n\
    \tscootnotify --help\n\
    \tscootnotify --version\n\
    \n\
    Commands:\n\
    \tdaemon\t\town org.freedesktop.Notifications and serve it\n\
    \tmsg\t\task the running daemon: list, dismiss, invoke, reload,\n\
    \t\t\tsubscribe, wait, version\n\
    \n\
    Flags:\n\
    \t--check\t\tvalidate the config file and exit (0 valid, 1 refused)\n\
    \t--config PATH\tread this config file instead of the default\n\
    \t--font PATH\tdraw with this font file instead of the first well-known one\n\
    \t--help\t\tprint this help and exit\n\
    \t--version\tprint the version and exit\n\
    \n\
    `msg wait` blocks until a live or arriving notification matches every\n\
    filter given (`--app` names its app exactly, `--summary-contains` a\n\
    substring of its summary), prints its JSON, and exits 0; on `--timeout`\n\
    (30 s when absent) it exits 1. `msg subscribe` prints the `subscribed`\n\
    reply and then one JSON event per line until the daemon closes the\n\
    connection.\n\
    \n\
    Exit statuses: 0 done, 1 failed, 2 misused, 3 the name is already owned.\n\
    The full reference is in the scootnotify section of the docs site.\n";

/// Parses `argv` (with `argv[0]` the binary name). `--help` and
/// `--version` win wherever they stand, as for the other shell apps; a
/// `TEXT`-shaped flag value is taken literally (whatever follows
/// `--config` is the path).
pub fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut args = Args {
        command: Command::Daemon,
        check: false,
        config: None,
        font: None,
        help: false,
        version: false,
    };
    // `--help`/`--version` anywhere win over every usage error, as for
    // the other shell apps: a user asking for help gets help.
    for arg in &argv[1..] {
        if arg == "--help" || arg == "-h" {
            args.help = true;
        } else if arg == "--version" || arg == "-V" {
            args.version = true;
        }
    }
    if args.help || args.version {
        return Ok(args);
    }
    let mut at = 1;
    if argv.get(at).is_some_and(|word| word == "daemon") {
        at += 1;
    } else if argv.get(at).is_some_and(|word| word == "msg") {
        args.command = Command::Msg(parse_msg(&argv[at + 1..])?);
        return Ok(args);
    } else if argv.get(at).is_some() {
        return Err(format!(
            "unknown command '{}' (they are: daemon, msg)",
            argv[at]
        ));
    } else {
        return Err(String::from(
            "a command is required (they are: daemon, msg)",
        ));
    }
    while let Some(word) = argv.get(at) {
        if word == "--check" {
            args.check = true;
            at += 1;
        } else if word == "--config" {
            let path = argv
                .get(at + 1)
                .ok_or_else(|| String::from("--config needs a value (the config file to read)"))?;
            args.config = Some(path.clone());
            at += 2;
        } else if word == "--font" {
            let path = argv
                .get(at + 1)
                .ok_or_else(|| String::from("--font needs a value (the font file to draw with)"))?;
            args.font = Some(path.clone());
            at += 2;
        } else {
            return Err(format!("unknown flag '{word}' (see --help)"));
        }
    }
    Ok(args)
}

/// Parses the words after `msg`.
fn parse_msg(words: &[String]) -> Result<Msg, String> {
    let command = words.first().map(String::as_str).ok_or_else(|| {
        String::from("a msg command is required (they are: list, dismiss, invoke, reload, subscribe, wait, version)")
    })?;
    let rest = &words[1..];
    match command {
        "list" => {
            let mut json = false;
            for word in rest {
                if word == "--json" {
                    json = true;
                } else {
                    return Err(format!(
                        "unknown flag '{word}' for `msg list` (it takes: --json)"
                    ));
                }
            }
            Ok(Msg::List { json })
        }
        "dismiss" => {
            let mut id = None;
            let mut all = false;
            for word in rest {
                if word == "--all" {
                    all = true;
                } else if id.is_none() {
                    match word.parse::<u32>().ok().filter(|id| *id != 0) {
                        Some(parsed) => id = Some(parsed),
                        None => {
                            return Err(format!(
                                "`msg dismiss` takes a notification id or `--all`, not '{word}'"
                            ));
                        }
                    }
                } else {
                    return Err(format!(
                        "`msg dismiss` takes a notification id or `--all`, not '{word}'"
                    ));
                }
            }
            match (id, all) {
                (Some(_), true) => Err(String::from(
                    "`msg dismiss` takes an id or `--all`, not both",
                )),
                (None, false) => Err(String::from(
                    "`msg dismiss` needs a notification id or `--all`",
                )),
                (id, all) => Ok(Msg::Dismiss { id, all }),
            }
        }
        "invoke" => {
            let id = rest.first().ok_or_else(|| {
                String::from("`msg invoke` needs a notification id (and an action, which defaults to `default`)")
            })?;
            let id = id
                .parse::<u32>()
                .ok()
                .filter(|id| *id != 0)
                .ok_or_else(|| {
                    format!("`msg invoke` needs a notification id from 1 to 4294967295, not '{id}'")
                })?;
            let action = rest
                .get(1)
                .cloned()
                .unwrap_or_else(|| String::from("default"));
            if rest.len() > 2 {
                return Err(format!(
                    "unknown argument '{}' for `msg invoke` (it takes: ID [ACTION])",
                    rest[2]
                ));
            }
            Ok(Msg::Invoke { id, action })
        }
        "reload" => {
            let mut look = false;
            for word in rest {
                if word == "--look" {
                    look = true;
                } else {
                    return Err(format!(
                        "unknown flag '{word}' for `msg reload` (it takes: --look)"
                    ));
                }
            }
            Ok(Msg::Reload { look })
        }
        "subscribe" => {
            let mut events = Vec::new();
            for word in rest {
                let kind = EventKind::parse(word).ok_or_else(|| {
                    format!("unknown event kind `{word}` (they are: notify, close, invoke)")
                })?;
                if !events.contains(&kind) {
                    events.push(kind);
                }
            }
            if events.is_empty() {
                events.extend(EventKind::ALL);
            }
            Ok(Msg::Subscribe { events })
        }
        "wait" => parse_wait(rest),
        "version" => {
            if let Some(word) = rest.first() {
                return Err(format!(
                    "unknown argument '{word}' for `msg version` (it takes none)"
                ));
            }
            Ok(Msg::Version)
        }
        _ => Err(format!(
            "unknown msg command '{command}' (they are: list, dismiss, invoke, reload, subscribe, wait, version)"
        )),
    }
}

/// Parses the words after `msg wait`.
fn parse_wait(words: &[String]) -> Result<Msg, String> {
    let mut app = None;
    let mut summary = None;
    let mut timeout_secs = DEFAULT_WAIT_TIMEOUT_SECS;
    let mut at = 0;
    while let Some(word) = words.get(at) {
        if word == "--app" {
            let value = words
                .get(at + 1)
                .ok_or_else(|| String::from("`--app` needs a value (the app name to wait for)"))?;
            app = Some(value.clone());
            at += 2;
        } else if word == "--summary-contains" {
            let value = words.get(at + 1).ok_or_else(|| {
                String::from("`--summary-contains` needs a value (the summary text to wait for)")
            })?;
            summary = Some(value.clone());
            at += 2;
        } else if word == "--timeout" {
            let value = words
                .get(at + 1)
                .ok_or_else(|| String::from("`--timeout` needs a value (seconds to wait)"))?;
            timeout_secs = value.parse::<u64>().map_err(|_| {
                format!("`--timeout` needs a whole number of seconds, not '{value}'")
            })?;
            at += 2;
        } else {
            return Err(format!(
                "unknown flag '{word}' for `msg wait` (it takes: --app, --summary-contains, --timeout)"
            ));
        }
    }
    Ok(Msg::Wait {
        app,
        summary,
        timeout_secs,
    })
}

#[cfg(test)]
mod tests;
