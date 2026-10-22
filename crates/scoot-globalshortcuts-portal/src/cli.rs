//! `scoot-globalshortcuts-portal` arguments: `daemon` is the process,
//! and nothing else ships (no control socket: the portal is the API).

/// What the binary should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Daemon,
}

/// The parsed arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub command: Command,
    pub help: bool,
    pub version: bool,
}

/// The `--help` text.
pub const HELP: &str = "\
scoot-globalshortcuts-portal: the GlobalShortcuts portal backend

Usage: scoot-globalshortcuts-portal [daemon] [--help] [--version]

Owns org.freedesktop.impl.portal.desktop.scoot and serves
org.freedesktop.impl.portal.GlobalShortcuts v2 at
/org/freedesktop/portal/desktop, registering shortcuts with the
compositor over scoot-ipc and turning its global-shortcut events into
Activated/Deactivated/ShortcutsChanged signals.

With no command, runs the daemon.
";

/// Parses `argv` (with `argv[0]` the program name). `Err` is the usage
/// error to print.
pub fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut command = None;
    let mut help = false;
    let mut version = false;
    for arg in &argv[1..] {
        match arg.as_str() {
            "daemon" if command.is_none() => command = Some(Command::Daemon),
            "--help" | "-h" => help = true,
            "--version" | "-V" => version = true,
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok(Args {
        command: command.unwrap_or(Command::Daemon),
        help,
        version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn bare_runs_the_daemon() {
        assert_eq!(
            parse_args(&argv(&["scoot-globalshortcuts-portal"])).unwrap(),
            Args {
                command: Command::Daemon,
                help: false,
                version: false,
            }
        );
    }

    #[test]
    fn unknown_is_an_error() {
        assert!(parse_args(&argv(&["scoot-globalshortcuts-portal", "frobnicate"])).is_err());
    }
}
