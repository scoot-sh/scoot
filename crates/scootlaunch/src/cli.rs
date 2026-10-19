//! The command line: `scootlaunch` (the panel), `scootlaunch --list
//! [--all] [--lenient] [--json]`, `scootlaunch --query TEXT [--all]
//! [--lenient] [--json]`, and `scootlaunch --check [--config PATH]`.
//!
//! Later tickets add `--run-id`, the dmenu contract and run mode
//! (`dev/design/scootlaunch.md`, Modes). Anything not listed here is a
//! loud error (exit 2, naming the flag and the supported list), never
//! silently ignored.

use std::fmt;

#[cfg(test)]
mod tests;

/// What the commands take. `list`/`query` are the headless agent
/// interface; bare (no command) is the panel. `all` shows `NoDisplay`
/// and `OnlyShowIn`/`NotShowIn`/`TryExec`-hidden entries too, with their
/// reason; `json` prints the agent JSON schema instead of text lines;
/// `lenient` accepts what GLib accepts for Exec quoting (single quotes,
/// unquoted reserved characters) instead of the spec's strict rule, for
/// real files that break it; `query` holds the search text, ranked as the
/// UI would rank it, frecency included. `font` names a `.ttf`/`.otf` file
/// for the panel (else the first well-known one); `config` overrides the
/// panel's own file; `check` validates that file and exits.
#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub list: bool,
    pub all: bool,
    pub lenient: bool,
    pub json: bool,
    pub help: bool,
    pub version: bool,
    pub query: Option<String>,
    pub font: Option<String>,
    pub config: Option<String>,
    pub check: bool,
}

pub const HELP: &str = "\
scootlaunch: a keyboard-first application launcher for Wayland

usage: scootlaunch [--font PATH] [--config PATH] [--lenient]
       scootlaunch --list [--all] [--lenient] [--json]
       scootlaunch --query TEXT [--all] [--lenient] [--json]
       scootlaunch --check [--config PATH]

  (no command) open the panel: the overlay exclusive-keyboard launcher,
    printing the chosen entry id (exit 0), or nothing on dismiss (exit 1)
  --font PATH    the panel's .ttf/.otf file (else the first well-known one)
  --config PATH  the panel's own launch.toml (else $XDG_CONFIG_HOME/scoot/launch.toml)
  --list       list indexed desktop entries (no compositor needed)
  --query TEXT rank entries for TEXT, as the UI would rank them
  --all        include hidden entries, with their reason
  --lenient    Exec quoting: accept what GLib accepts, not just the spec
  --json       print the agent JSON schema (id, name, exec argv, ...) array
  --check      validate the panel's config file and exit
  --help       print this text
  --version    print the version

TEXT is the next argument after --query, whatever it holds. --font and
--config are panel options; --all, --lenient and --json modify --list
and --query (--lenient also shapes the panel's index).
";

/// A usage error. Printed by the caller; exit status 2.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// A flag this ticket does not take, with the supported list.
    UnknownFlag(String),
    /// A positional argument, which no command takes.
    UnexpectedArg(String),
    /// A flag without the command it modifies (`--all`/`--json` without
    /// `--list` or `--query`; `--font` without the panel; `--config`
    /// without the panel or `--check`).
    NeedsCommand(String),
    /// `--query`, `--font` or `--config` with no value after it.
    NeedsValue(String),
    /// Two commands together (`--list` with `--query`, `--check` with
    /// either): two commands, not modifiers.
    ConflictingCommands,
}

const USAGE: &str = "usage is scootlaunch [--font PATH] [--config PATH] [--lenient], scootlaunch --list [--all] [--lenient] [--json], scootlaunch --query TEXT [--all] [--lenient] [--json] or scootlaunch --check [--config PATH]";

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnknownFlag(flag) => write!(
                f,
                "unknown flag {flag}: supported flags are --list --query --all --lenient --json --font --config --check --help --version"
            ),
            Error::UnexpectedArg(arg) => write!(f, "unexpected argument {arg}: {USAGE}"),
            Error::NeedsCommand(flag) => {
                write!(f, "{flag} without the command it modifies: {USAGE}")
            }
            Error::NeedsValue(flag) => {
                write!(f, "{flag} needs a value: {USAGE}")
            }
            Error::ConflictingCommands => {
                write!(f, "cannot combine --list and --query: {USAGE}")
            }
        }
    }
}

/// Parses `argv` (including `argv[0]`, the program name). Every flag is
/// a whole argument; `--flag=value` is not a flag but an unknown one.
/// `--query` takes the next argument as its text, whatever it holds.
pub fn parse_args(argv: &[String]) -> Result<Args, Error> {
    let mut args = Args {
        list: false,
        all: false,
        lenient: false,
        json: false,
        help: false,
        version: false,
        query: None,
        font: None,
        config: None,
        check: false,
    };
    if argv.len() < 2 {
        // Bare: the panel.
        return Ok(args);
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
                    query: None,
                    font: None,
                    config: None,
                    check: false,
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
                    query: None,
                    font: None,
                    config: None,
                    check: false,
                });
            }
            _ => {}
        }
    }
    let mut index = 1;
    while index < argv.len() {
        match argv[index].as_str() {
            "--list" => args.list = true,
            "--query" => {
                index += 1;
                match argv.get(index) {
                    Some(text) => args.query = Some(text.clone()),
                    None => return Err(Error::NeedsValue("--query".to_string())),
                }
            }
            "--all" => args.all = true,
            "--lenient" => args.lenient = true,
            "--json" => args.json = true,
            "--check" => args.check = true,
            "--font" => {
                index += 1;
                match argv.get(index) {
                    Some(path) => args.font = Some(path.clone()),
                    None => return Err(Error::NeedsValue("--font".to_string())),
                }
            }
            "--config" => {
                index += 1;
                match argv.get(index) {
                    Some(path) => args.config = Some(path.clone()),
                    None => return Err(Error::NeedsValue("--config".to_string())),
                }
            }
            // Reached only when the pre-scan above did not fire, so
            // these arms never run; kept explicit so a future flag
            // reordering cannot route `--help` into the unknown-flag
            // error below.
            "--help" | "-h" => args.help = true,
            "--version" | "-V" => args.version = true,
            flag if flag.starts_with('-') => return Err(Error::UnknownFlag(flag.to_string())),
            other => return Err(Error::UnexpectedArg(other.to_string())),
        }
        index += 1;
    }
    if args.list && args.query.is_some() {
        return Err(Error::ConflictingCommands);
    }
    if args.check && (args.list || args.query.is_some()) {
        return Err(Error::ConflictingCommands);
    }
    // `--font` is a panel option: it shapes nothing headless.
    if args.font.is_some() && (args.list || args.query.is_some() || args.check) {
        return Err(Error::NeedsCommand("--font".to_string()));
    }
    // `--config` names the panel's file: `--check` takes it, the panel
    // takes it, the headless commands do not.
    if args.config.is_some() && (args.list || args.query.is_some()) {
        return Err(Error::NeedsCommand("--config".to_string()));
    }
    for flag in ["--all", "--json"] {
        let set = match flag {
            "--all" => args.all,
            _ => args.json,
        };
        if set && !args.list && args.query.is_none() {
            return Err(Error::NeedsCommand(flag.to_string()));
        }
    }
    // `--lenient` modifies the headless commands and the panel alike; a
    // bare `--lenient` is a strictness flag for the panel's own index.
    Ok(args)
}
