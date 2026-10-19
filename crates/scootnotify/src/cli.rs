//! `scootnotify daemon [--check]`: the command line.
//!
//! One subcommand in this ticket (`daemon`; `msg` arrives with the
//! control socket in `notify-control`): it either checks the config and
//! exits, or owns `org.freedesktop.Notifications` until killed. Exit
//! statuses: 0 done, 1 failed, 2 misused, 3 the name is owned (the unit's
//! `RestartPreventExitStatus=3` turns that into a stopped unit, never a
//! restart loop).

/// What `daemon` runs with: everything the usage below promises.
#[derive(Debug, PartialEq, Eq)]
pub struct Args {
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

/// The usage both `--help` and the misuse refusal print.
pub const HELP: &str = "scootnotify: the notification daemon\n\
    \n\
    Usage:\n\
    \tscootnotify daemon [--check] [--config PATH] [--font PATH]\n\
    \tscootnotify --help\n\
    \tscootnotify --version\n\
    \n\
    Commands:\n\
    \tdaemon\t\town org.freedesktop.Notifications and serve it\n\
    \n\
    Flags:\n\
    \t--check\t\tvalidate the config file and exit (0 valid, 1 refused)\n\
    \t--config PATH\tread this config file instead of the default\n\
    \t--font PATH\tdraw with this font file instead of the first well-known one\n\
    \t--help\t\tprint this help and exit\n\
    \t--version\tprint the version and exit\n\
    \n\
    Exit statuses: 0 done, 1 failed, 2 misused, 3 the name is already owned.\n\
    The full reference is in the scootnotify section of the docs site.\n";

/// Parses `argv` (with `argv[0]` the binary name). `--help` and
/// `--version` win wherever they stand, as for the other shell apps; a
/// `TEXT`-shaped flag value is taken literally (whatever follows
/// `--config` is the path).
pub fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut args = Args {
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
    } else if argv.get(at).is_some() {
        return Err(format!(
            "unknown command '{}' (only 'daemon' serves yet; 'msg' arrives with the control socket)",
            argv[at]
        ));
    } else {
        return Err(String::from(
            "a command is required (only 'daemon' serves yet)",
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

#[cfg(test)]
mod tests;
