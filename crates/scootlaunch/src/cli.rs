//! The command line: `scootlaunch` (the panel), `scootlaunch --list
//! [--all] [--lenient] [--json]`, `scootlaunch --query TEXT [--all]
//! [--lenient] [--json]`, `scootlaunch --check [--config PATH]`,
//! `scootlaunch --dmenu [flags]` (lines on stdin, the pick on stdout)
//! and `scootlaunch --run` (executables on `$PATH`).
//!
//! Anything not listed here is a loud error (exit 2, naming the flag
//! and the supported list), never silently ignored. A value flag takes
//! its value attached (`--prompt=TEXT`) or as the next argument
//! (`--prompt TEXT`); a bool flag with `=...` is not that flag
//! (`--json=true` is unknown), so a misparsed value-taking flag can
//! never silently eat the next argument the way bare `--json true`
//! would.

use std::fmt;

use scootui::color::Color;

#[cfg(test)]
mod tests;

/// The command line's modes: at most one per invocation. The bare
/// panel, `--list`, `--query`, `--check`, `--dmenu` and `--run` are
/// the commands; everything else modifies one of them.
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
    /// `--dmenu`: lines on stdin, the pick on stdout (exit 0), Escape
    /// exits 1, any other flag exits 2. A dmenu pick prints and
    /// launches nothing.
    pub dmenu: bool,
    /// `--run` (alias `--list-executables-in-path`): the panel over
    /// executables on `$PATH`. A run pick launches, like an apps pick.
    pub run: bool,
    /// `--prompt TEXT`: the prompt drawn before the entry (`> ` when
    /// neither `--prompt` nor `--prompt-only` names one). Dmenu only.
    pub prompt: Option<String>,
    /// `--prompt-only TEXT`: an input field with no list (a password
    /// prompt, a WiFi key): stdin is not read. Dmenu only.
    pub prompt_only: Option<String>,
    /// `--password`: mask the entry with `*`. The secret is written
    /// to stdout only on Enter. Dmenu only.
    pub password: bool,
    /// `--no-run-if-empty`: empty stdin exits 1 without opening.
    /// Dmenu only.
    pub no_run_if_empty: bool,
    /// `--only-match`: Enter with no match does nothing (without it
    /// the typed text prints). Ctrl+Enter still prints the typed
    /// text. Dmenu only.
    pub only_match: bool,
    /// `--index`: print the 0-based stdin index of the pick instead
    /// of its line. Dmenu only.
    pub index: bool,
    /// `--placeholder TEXT`: the dim hint while the entry is empty
    /// (empty by default). Dmenu only.
    pub placeholder: Option<String>,
    /// `--layer overlay`: accepted; the panel is always `overlay`.
    pub layer: Option<String>,
    /// The seven `rrggbbaa` overrides, above the user's file and the
    /// look: `--background-color --text-color --border-color
    /// --selection-color --selection-text-color --match-color
    /// --prompt-color`. Only `background` may carry a non-opaque
    /// alpha (it sets the panel's opacity); any other translucent
    /// color is refused loudly.
    pub colors: ColorOverrides,
}

/// The seven per-invocation color overrides: each `rrggbbaa`, parsed.
/// `background` keeps its alpha (the panel's opacity); the rest must
/// be opaque (`...ff`), since the panel draws those roles solid.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ColorOverrides {
    pub background: Option<(Color, u8)>,
    pub text: Option<Color>,
    pub border: Option<Color>,
    pub selection: Option<Color>,
    pub selection_text: Option<Color>,
    pub r#match: Option<Color>,
    pub prompt: Option<Color>,
}

impl ColorOverrides {
    fn is_empty(&self) -> bool {
        self.background.is_none()
            && self.text.is_none()
            && self.border.is_none()
            && self.selection.is_none()
            && self.selection_text.is_none()
            && self.r#match.is_none()
            && self.prompt.is_none()
    }
}

pub const HELP: &str = "\
scootlaunch: a keyboard-first application launcher for Wayland

usage: scootlaunch [--font PATH] [--config PATH] [--lenient]
       scootlaunch --list [--all] [--lenient] [--json]
       scootlaunch --query TEXT [--all] [--lenient] [--json]
       scootlaunch --check [--config PATH]
       scootlaunch --dmenu [--prompt TEXT] [--prompt-only TEXT] [--password]
                     [--no-run-if-empty] [--only-match] [--index]
                     [--placeholder TEXT] [--layer overlay]
                     [--background-color RRGGBBAA ...]
       scootlaunch --run

  (no command) open the panel: the overlay exclusive-keyboard launcher,
    launching the chosen entry (exit 0), or nothing on dismiss (exit 1)
  --font PATH    the panel's .ttf/.otf file (else the first well-known one)
  --config PATH  the panel's own launch.toml (else $XDG_CONFIG_HOME/scoot/launch.toml)
  --list       list indexed desktop entries (no compositor needed)
  --query TEXT rank entries for TEXT, as the UI would rank them
  --all        include hidden entries, with their reason
  --lenient    Exec quoting: accept what GLib accepts, not just the spec
  --json       print the agent JSON schema (id, name, exec argv, ...) array
  --check      validate the panel's config file and exit
  --dmenu      lines on stdin, the pick on stdout (exit 0); Escape exits 1;
                 a dmenu pick prints and launches nothing
  --prompt TEXT        the prompt before the entry (default '> ')
  --prompt-only TEXT   an input field with no list; stdin is not read
  --password           mask the entry ('*'); the secret prints on Enter only
  --no-run-if-empty    empty stdin: exit 1 without opening
  --only-match         Enter with no match does nothing (else the typed text prints)
  --index              print the 0-based stdin index of the pick, not its line
  --placeholder TEXT   the dim hint while the entry is empty (default empty)
  --run                the panel over executables on $PATH (alias:
                         --list-executables-in-path, fuzzel's flag)
  --layer overlay      accepted; the panel is always overlay
  --background-color, --text-color, --border-color, --selection-color,
    --selection-text-color, --match-color, --prompt-color (rrggbbaa):
    per-invocation overrides above the user's file and the look
  --help       print this text
  --version    print the version

TEXT is the next argument after --query, whatever it holds. --prompt,
--prompt-only, --placeholder, --layer and the colors take their value
attached (--prompt=TEXT) or as the next argument (--prompt TEXT).
--font and --config are panel options; --all, --json and --lenient
modify --list and --query (--lenient also shapes the panel's index).
The dmenu flags need --dmenu; the colors and --layer shape the panel,
--run and --dmenu alike. One command per invocation.
";

/// A usage error. Printed by the caller; exit status 2.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// A flag this command does not take, with the supported list.
    UnknownFlag(String),
    /// A positional argument, which no command takes.
    UnexpectedArg(String),
    /// A flag without the command it modifies (`--all`/`--json` without
    /// `--list` or `--query`; `--font` without the panel; `--config`
    /// without the panel or `--check`; the dmenu flags without
    /// `--dmenu`; the colors/`--layer` without a panel).
    NeedsCommand(String),
    /// A value flag (`--query`, `--font`, `--config`, `--prompt`,
    /// `--prompt-only`, `--placeholder`, `--layer`, a color) with no
    /// value after it.
    NeedsValue(String),
    /// Two commands together (any two of `--list`, `--query`,
    /// `--check`, `--dmenu`, `--run`), or `--prompt` with
    /// `--prompt-only`: two commands, not modifiers.
    ConflictingCommands,
    /// A value flag with a value outside its grammar: `--layer`
    /// past `top`/`overlay` or a color past `rrggbbaa`. The
    /// flag, the value, what was expected.
    InvalidValue {
        flag: String,
        value: String,
        expected: &'static str,
    },
}

const USAGE: &str = "usage is scootlaunch [--font PATH] [--config PATH] [--lenient], scootlaunch --list [--all] [--lenient] [--json], scootlaunch --query TEXT [--all] [--lenient] [--json], scootlaunch --check [--config PATH], scootlaunch --dmenu [flags] or scootlaunch --run";

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnknownFlag(flag) => write!(
                f,
                "unknown flag {flag}: supported flags are --list --query --all --lenient --json --font --config --check --dmenu --run --prompt --prompt-only --password --no-run-if-empty --only-match --index --placeholder --layer --background-color --text-color --border-color --selection-color --selection-text-color --match-color --prompt-color --help --version"
            ),
            Error::UnexpectedArg(arg) => write!(f, "unexpected argument {arg}: {USAGE}"),
            Error::NeedsCommand(flag) => {
                write!(f, "{flag} without the command it modifies: {USAGE}")
            }
            Error::NeedsValue(flag) => {
                write!(f, "{flag} needs a value: {USAGE}")
            }
            Error::ConflictingCommands => {
                write!(
                    f,
                    "cannot combine --list, --query, --check, --dmenu, --run or --prompt with --prompt-only: {USAGE}"
                )
            }
            Error::InvalidValue {
                flag,
                value,
                expected,
            } => {
                write!(
                    f,
                    "invalid value {value} for {flag}: expected {expected}: {USAGE}"
                )
            }
        }
    }
}

/// Parses `argv` (including `argv[0]`, the program name). Every flag is
/// a whole argument, except the value flags (`--prompt`,
/// `--prompt-only`, `--placeholder`, `--layer`, the seven colors),
/// which also take `--flag=value`. `--query` takes the next argument
/// as its text, whatever it holds. `--password` never eats the next
/// argument, so a following value is never eaten by mistake.
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
        dmenu: false,
        run: false,
        prompt: None,
        prompt_only: None,
        password: false,
        no_run_if_empty: false,
        only_match: false,
        index: false,
        placeholder: None,
        layer: None,
        colors: ColorOverrides::default(),
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
                    help: true,
                    ..empty_args()
                });
            }
            "--version" | "-V" => {
                return Ok(Args {
                    version: true,
                    ..empty_args()
                });
            }
            _ => {}
        }
    }
    let mut index = 1;
    while index < argv.len() {
        // A value flag in `--flag=value` form: split once, so a value
        // holding `=` (a prompt, a placeholder) rides through whole.
        let (name, attached) = match argv[index].split_once('=') {
            Some((name, value)) if name.starts_with("--") => {
                (name.to_string(), Some(value.to_string()))
            }
            _ => (argv[index].clone(), None),
        };
        match name.as_str() {
            "--list" => {
                no_attached(&argv[index], attached)?;
                args.list = true;
            }
            "--query" => {
                // `--query` takes the next argument, whatever it
                // holds; `--query=text` is not the flag (a query may
                // itself hold `=`, taken in the space form).
                if attached.is_some() {
                    return Err(Error::UnknownFlag(argv[index].clone()));
                }
                index += 1;
                match argv.get(index) {
                    Some(text) => args.query = Some(text.clone()),
                    None => return Err(Error::NeedsValue("--query".to_string())),
                }
            }
            "--all" => {
                no_attached(&argv[index], attached)?;
                args.all = true;
            }
            "--lenient" => {
                no_attached(&argv[index], attached)?;
                args.lenient = true;
            }
            "--json" => {
                no_attached(&argv[index], attached)?;
                args.json = true;
            }
            "--check" => {
                no_attached(&argv[index], attached)?;
                args.check = true;
            }
            "--font" => {
                args.font = Some(take_value(argv, &mut index, attached, "--font")?);
            }
            "--config" => {
                args.config = Some(take_value(argv, &mut index, attached, "--config")?);
            }
            "--dmenu" => {
                no_attached(&argv[index], attached)?;
                args.dmenu = true;
            }
            "--run" | "--list-executables-in-path" => {
                no_attached(&argv[index], attached)?;
                args.run = true;
            }
            "--prompt" => {
                args.prompt = Some(take_value(argv, &mut index, attached, "--prompt")?);
            }
            "--prompt-only" => {
                args.prompt_only = Some(take_value(argv, &mut index, attached, "--prompt-only")?);
            }
            "--placeholder" => {
                args.placeholder = Some(take_value(argv, &mut index, attached, "--placeholder")?);
            }
            "--layer" => {
                let value = take_value(argv, &mut index, attached, "--layer")?;
                if value != "top" && value != "overlay" {
                    return Err(Error::InvalidValue {
                        flag: "--layer".to_string(),
                        value,
                        expected: "top or overlay",
                    });
                }
                args.layer = Some(value);
            }
            "--password" => {
                no_attached(&argv[index], attached)?;
                args.password = true;
            }
            "--no-run-if-empty" => {
                no_attached(&argv[index], attached)?;
                args.no_run_if_empty = true;
            }
            "--only-match" => {
                no_attached(&argv[index], attached)?;
                args.only_match = true;
            }
            "--index" => {
                no_attached(&argv[index], attached)?;
                args.index = true;
            }
            "--background-color" => {
                args.colors.background = Some(parse_rgba(take_value(
                    argv,
                    &mut index,
                    attached,
                    "--background-color",
                )?)?);
            }
            "--text-color" => {
                args.colors.text = Some(parse_opaque(
                    &take_value(argv, &mut index, attached, "--text-color")?,
                    "--text-color",
                )?);
            }
            "--border-color" => {
                args.colors.border = Some(parse_opaque(
                    &take_value(argv, &mut index, attached, "--border-color")?,
                    "--border-color",
                )?);
            }
            "--selection-color" => {
                args.colors.selection = Some(parse_opaque(
                    &take_value(argv, &mut index, attached, "--selection-color")?,
                    "--selection-color",
                )?);
            }
            "--selection-text-color" => {
                args.colors.selection_text = Some(parse_opaque(
                    &take_value(argv, &mut index, attached, "--selection-text-color")?,
                    "--selection-text-color",
                )?);
            }
            "--match-color" => {
                args.colors.r#match = Some(parse_opaque(
                    &take_value(argv, &mut index, attached, "--match-color")?,
                    "--match-color",
                )?);
            }
            "--prompt-color" => {
                args.colors.prompt = Some(parse_opaque(
                    &take_value(argv, &mut index, attached, "--prompt-color")?,
                    "--prompt-color",
                )?);
            }
            // Reached only when the pre-scan above did not fire, so
            // these arms never run; kept explicit so a future flag
            // reordering cannot route `--help` into the unknown-flag
            // error below.
            "--help" | "-h" => args.help = true,
            "--version" | "-V" => args.version = true,
            flag if flag.starts_with('-') => {
                return Err(Error::UnknownFlag(argv[index].clone()));
            }
            other => return Err(Error::UnexpectedArg(other.to_string())),
        }
        index += 1;
    }
    let mut commands = 0;
    if args.list {
        commands += 1;
    }
    if args.query.is_some() {
        commands += 1;
    }
    if args.check {
        commands += 1;
    }
    if args.dmenu {
        commands += 1;
    }
    if args.run {
        commands += 1;
    }
    if commands > 1 {
        return Err(Error::ConflictingCommands);
    }
    // `--prompt` and `--prompt-only` are two prompts, not one with a
    // modifier: the prompt-only value is the prompt.
    if args.prompt.is_some() && args.prompt_only.is_some() {
        return Err(Error::ConflictingCommands);
    }
    // `--font` is a panel option: it shapes nothing headless.
    if args.font.is_some() && (args.list || args.query.is_some() || args.check) {
        return Err(Error::NeedsCommand("--font".to_string()));
    }
    // `--config` names the panel's file: the panel modes and `--check`
    // take it, the headless commands do not.
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
    // The dmenu input flags need `--dmenu`.
    for (flag, set) in [
        ("--prompt", args.prompt.is_some()),
        ("--prompt-only", args.prompt_only.is_some()),
        ("--password", args.password),
        ("--no-run-if-empty", args.no_run_if_empty),
        ("--only-match", args.only_match),
        ("--index", args.index),
        ("--placeholder", args.placeholder.is_some()),
    ] {
        if set && !args.dmenu {
            return Err(Error::NeedsCommand(flag.to_string()));
        }
    }
    // The colors and `--layer` shape the panel: the bare panel,
    // `--run` and `--dmenu` take them, the headless commands and
    // `--check` do not.
    if (!args.colors.is_empty() || args.layer.is_some())
        && (args.list || args.query.is_some() || args.check)
    {
        return Err(Error::NeedsCommand("--background-color".to_string()));
    }
    // `--lenient` modifies the headless commands and the panel's own
    // index alike; a bare `--lenient` is the panel's strictness flag.
    // With `--dmenu` or `--run` there is no desktop index to shape,
    // and it rides along harmlessly.
    Ok(args)
}

fn empty_args() -> Args {
    Args {
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
        dmenu: false,
        run: false,
        prompt: None,
        prompt_only: None,
        password: false,
        no_run_if_empty: false,
        only_match: false,
        index: false,
        placeholder: None,
        layer: None,
        colors: ColorOverrides::default(),
    }
}

/// A bool flag never takes `=...`: `--json=true` is not `--json`,
/// so a value flag's misparse cannot silently eat the next argument.
fn no_attached(full: &str, attached: Option<String>) -> Result<(), Error> {
    if attached.is_some() {
        return Err(Error::UnknownFlag(full.to_string()));
    }
    Ok(())
}

/// The value of a value flag: attached (`--prompt=TEXT`) or the next
/// argument (`--prompt TEXT`), whatever it holds. Missing either way
/// is [`Error::NeedsValue`], never the next flag silently defaulted.
fn take_value(
    argv: &[String],
    index: &mut usize,
    attached: Option<String>,
    flag: &str,
) -> Result<String, Error> {
    if let Some(value) = attached {
        return Ok(value);
    }
    *index += 1;
    match argv.get(*index) {
        Some(value) => Ok(value.clone()),
        None => Err(Error::NeedsValue(flag.to_string())),
    }
}

/// Parses `rrggbbaa` (eight hex digits, either case, no prefix),
/// keeping the alpha: the background role honors it as the panel's
/// opacity.
fn parse_rgba(text: String) -> Result<(Color, u8), Error> {
    let bytes = text.as_bytes();
    if bytes.len() != 8 || !bytes.iter().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::InvalidValue {
            flag: "--background-color".to_string(),
            value: text,
            expected: "rrggbbaa (eight hex digits)",
        });
    }
    let byte = |high: u8, low: u8| {
        let digit = |char: u8| {
            if char.is_ascii_digit() {
                char - b'0'
            } else {
                char.to_ascii_lowercase() - b'a' + 10
            }
        };
        digit(high) << 4 | digit(low)
    };
    Ok((
        Color {
            r: byte(bytes[0], bytes[1]),
            g: byte(bytes[2], bytes[3]),
            b: byte(bytes[4], bytes[5]),
        },
        byte(bytes[6], bytes[7]),
    ))
}

/// Parses `rrggbbaa` for a solid role: the alpha must be opaque
/// (`...ff`), since the panel draws these roles without blending.
/// The flag names the error, so `--text-color=...` reads as itself.
fn parse_opaque(text: &str, flag: &str) -> Result<Color, Error> {
    match parse_rgba(text.to_string()) {
        Ok((color, 0xff)) => Ok(color),
        Ok((_, _)) => Err(Error::InvalidValue {
            flag: flag.to_string(),
            value: text.to_string(),
            expected: "an opaque rrggbbaa (...ff; only --background-color may be translucent)",
        }),
        Err(_) => Err(Error::InvalidValue {
            flag: flag.to_string(),
            value: text.to_string(),
            expected: "rrggbbaa (eight hex digits)",
        }),
    }
}
