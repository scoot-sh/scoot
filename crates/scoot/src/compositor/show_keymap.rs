//! What `Super+Shift+/` (`Action::ShowKeymap`) opens: the live keymap in a
//! terminal, with a pager when one is there.
//!
//! The core only says *what* to open (see `scoot_core::Effect::ShowKeymap`)
//! -- every program name lives here, in the crate that actually spawns. The
//! command is built at fire time from the live table, so it tracks the
//! session rather than the defaults:
//!
//! 1. When `xdg-terminal-exec` is on `PATH`, the keymap opens through it --
//!    the desktop-level "user's preferred terminal" -- as
//!    `xdg-terminal-exec sh -c '<script>'`.
//! 2. Otherwise the terminal is the live table's `Super+Return` bind when
//!    that bind is a `spawn`: its argv verbatim (so `ghostty --class x`
//!    keeps its own arguments), plus the words that terminal needs before a
//!    command to run (see `KNOWN_TERMINALS`). A terminal not in that table
//!    has no known-safe spelling -- guessing flags at an unknown terminal
//!    risks running the pager script as the terminal's own options -- so it
//!    falls through like a non-terminal `Super+Return` bind does.
//! 3. Otherwise `foot`, the same terminal the default `Super+Return` bind
//!    spawns.
//!
//! The payload is always `sh -c '<script>'`: no terminal runs a pipeline
//! itself, so the pipe into the pager goes through an explicit shell. The
//! script degrades when `less` is missing (see `KEYMAP_SCRIPT`) rather than
//! leaving an empty terminal, and the shell warns once -- naming what was
//! tried -- when even the `foot` fallback is not on `PATH`.
//!
//! A cold path (one keypress per opening): the `PATH` walks and the small
//! allocations below cost nothing here.
//!
//! Two caveats, both from reusing the user's own argv verbatim: a
//! `Super+Return` bind that already carries the terminal's exec words (e.g.
//! `spawn ghostty -e`) gets them twice, and a bind whose `argv[0]` is a
//! wrapper script is looked up under the wrapper's name, not what it runs.
//! Spell the bind as the terminal plus its own options and both go away.

use std::path::Path;

use scoot_core::Action;

use super::keybindings::{Bound, Keybindings, Modifiers};
use smithay::input::keyboard::Keysym;

/// The `sh -c` script every show-keymap spawn runs: the live keymap through
/// the pager when one is there, plain output plus a wait for Enter when
/// `less` is missing (rather than an empty terminal that opens and stays
/// blank).
///
/// The `read` names `/dev/tty` explicitly: the script's own stdin is the
/// pipe from `scoot`, already at end-of-file once `cat` has run, so a bare
/// `read` would return at once without waiting. `less` needs no such help --
/// it opens the terminal itself, which is why the plain pipeline works when
/// it is there.
pub const KEYMAP_SCRIPT: &str = "scoot msg binds | if command -v less >/dev/null 2>&1; then less; else cat; printf '%s' 'press Enter to close: '; read _ < /dev/tty; fi";

/// The `xdg-terminal-exec` convention: the desktop-level preferred terminal,
/// used before anything derived from the keymap when it is on `PATH`.
pub const XDG_TERMINAL_EXEC: &str = "xdg-terminal-exec";

/// The last-resort terminal: the same one the default `Super+Return` bind
/// spawns, so a session with no `xdg-terminal-exec` and no terminal on
/// `Super+Return` still opens the keymap wherever the defaults do.
pub const FALLBACK_TERMINAL: &str = "foot";

/// How to run a command in a known terminal: the words between the
/// terminal's own argv and the payload. `foot` and `kitty` run a trailing
/// command directly; the `-e` family needs its flag; `gnome-terminal`
/// separates with `--`; `wezterm` takes the `start` subcommand.
const KNOWN_TERMINALS: &[(&str, &[&str])] = &[
    ("foot", &[]),
    ("footclient", &[]),
    ("kitty", &[]),
    ("ghostty", &["-e"]),
    ("alacritty", &["-e"]),
    ("konsole", &["-e"]),
    ("xterm", &["-e"]),
    ("gnome-terminal", &["--"]),
    ("wezterm", &["start", "--"]),
];

/// The command opening the live keymap for `table`: the terminal
/// [`super_return_terminal`] names plus its exec words, or
/// [`FALLBACK_TERMINAL`], or [`XDG_TERMINAL_EXEC`] when `xdg_terminal_exec`
/// (probed by the caller -- see [`program_on_path`]) is present.
///
/// `xdg_terminal_exec` is a parameter rather than probed here so tests can
/// pin both resolutions without touching `PATH`.
pub fn show_keymap_command(table: &Keybindings, xdg_terminal_exec: bool) -> Vec<String> {
    let mut command: Vec<String> = if xdg_terminal_exec {
        vec![XDG_TERMINAL_EXEC.to_owned()]
    } else if let Some(argv) = super_return_terminal(table)
        && let Some(exec) = exec_words(&argv[0])
    {
        let mut command = argv;
        command.extend(exec.iter().map(|word| (*word).to_owned()));
        command
    } else {
        vec![FALLBACK_TERMINAL.to_owned()]
    };
    command.extend(["sh".to_owned(), "-c".to_owned(), KEYMAP_SCRIPT.to_owned()]);
    command
}

/// The live table's `Super+Return` bind when it is a `spawn`: its argv, used
/// verbatim as the terminal's own (see the module doc). Anything else -- a
/// rebind to another action, an unbound `Super+Return`, an empty `spawn` --
/// is `None`, and the caller falls back to [`FALLBACK_TERMINAL`].
pub fn super_return_terminal(table: &Keybindings) -> Option<Vec<String>> {
    match table.match_key(
        Keysym::Return,
        Modifiers {
            super_: true,
            ..Modifiers::default()
        },
    ) {
        Some((Bound::Action(Action::Spawn(argv)), _)) if !argv.is_empty() => Some(argv),
        _ => None,
    }
}

/// Whether `program` resolves on `PATH`: a walk, never an exec, so probing
/// has no side effects (the same shape as the XWayland live suites' probe).
/// A `program` naming a path directly (`/usr/bin/foot`) is checked as a
/// file instead.
pub fn program_on_path(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// The exec words for `argv0` when it names a [`KNOWN_TERMINALS`] terminal
/// (compared by file name, so a full path still matches): `None` for an
/// unknown terminal, which the caller falls back past rather than guessing
/// flags for.
fn exec_words(argv0: &str) -> Option<&'static [&'static str]> {
    let name = Path::new(argv0)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(argv0);
    KNOWN_TERMINALS
        .iter()
        .find(|(terminal, _)| *terminal == name)
        .map(|(_, exec)| *exec)
}

#[cfg(test)]
mod tests {
    use super::super::keybindings::{BindFlags, Modifiers};
    use super::*;
    use scoot_core::Action;

    const SUPER: Modifiers = Modifiers {
        super_: true,
        shift: false,
        ctrl: false,
        alt: false,
    };
    const SUPER_SHIFT: Modifiers = Modifiers {
        super_: true,
        shift: true,
        ctrl: false,
        alt: false,
    };

    /// `show_keymap_command`'s payload: `sh -c` carrying the viewer script.
    fn payload_of(command: &[String]) -> &[String] {
        let script = command
            .last()
            .expect("the command ends in the viewer script");
        assert_eq!(script, KEYMAP_SCRIPT, "unexpected script: {command:?}");
        let sh = &command[command.len() - 3..command.len() - 1];
        assert_eq!(sh, ["sh", "-c"], "unexpected payload: {command:?}");
        &command[..command.len() - 3]
    }

    #[test]
    fn the_default_table_opens_in_foot() {
        // The default `Super+Return` bind spawns `foot`, so the keymap opens
        // there too -- the pin that keeps the two together, read off the
        // live table rather than a copied literal.
        let command = show_keymap_command(&Keybindings::default(), false);
        assert_eq!(payload_of(&command), ["foot"]);
    }

    #[test]
    fn a_rebound_super_return_makes_show_keymap_spawn_that_terminal() {
        // `ghostty --class x`: the rebind's argv verbatim, then ghostty's
        // exec flag, then the payload.
        let mut table = Keybindings::default();
        table.insert(
            SUPER,
            Keysym::Return,
            Bound::Action(Action::Spawn(vec![
                "ghostty".into(),
                "--class".into(),
                "x".into(),
            ])),
            BindFlags::default(),
        );
        let command = show_keymap_command(&table, false);
        assert_eq!(
            payload_of(&command),
            ["ghostty", "--class", "x", "-e"],
            "the terminal's own arguments must survive: {command:?}"
        );
    }

    #[test]
    fn no_spawn_on_super_return_falls_back_to_foot() {
        // A `Super+Return` rebound to a non-spawn action is no terminal at
        // all: the keymap still opens, in the fallback.
        let mut table = Keybindings::default();
        table.insert(
            SUPER,
            Keysym::Return,
            Bound::Action(Action::CloseFocused),
            BindFlags::default(),
        );
        assert_eq!(payload_of(&show_keymap_command(&table, false)), ["foot"]);
        // And an unbound `Super+Return` falls the same way.
        let mut unbound = Keybindings::default();
        unbound.remove(SUPER, Keysym::Return);
        assert_eq!(payload_of(&show_keymap_command(&unbound, false)), ["foot"]);
    }

    #[test]
    fn an_unknown_terminal_falls_back_to_foot() {
        // No guessing flags at a terminal the table does not know: the
        // predictable fallback, not a possibly-wrong direct append.
        let mut table = Keybindings::default();
        table.insert(
            SUPER,
            Keysym::Return,
            Bound::Action(Action::Spawn(vec!["myterm".into()])),
            BindFlags::default(),
        );
        assert_eq!(payload_of(&show_keymap_command(&table, false)), ["foot"]);
    }

    #[test]
    fn xdg_terminal_exec_wins_when_present() {
        // Even over an explicit rebind: the desktop-level preference is the
        // stronger signal of "the user's terminal".
        let mut table = Keybindings::default();
        table.insert(
            SUPER,
            Keysym::Return,
            Bound::Action(Action::Spawn(vec!["ghostty".into()])),
            BindFlags::default(),
        );
        let command = show_keymap_command(&table, true);
        assert_eq!(payload_of(&command), ["xdg-terminal-exec"]);
    }

    #[test]
    fn every_known_terminal_spells_its_exec_words() {
        // One row per table entry: the flag (or none) between the terminal
        // and the payload. A full path matches by file name.
        for (terminal, exec) in [
            ("foot", vec![]),
            ("kitty", vec![]),
            ("ghostty", vec!["-e"]),
            ("alacritty", vec!["-e"]),
            ("wezterm", vec!["start", "--"]),
            ("/usr/bin/gnome-terminal", vec!["--"]),
        ] {
            let mut table = Keybindings::default();
            table.insert(
                SUPER,
                Keysym::Return,
                Bound::Action(Action::Spawn(vec![terminal.into()])),
                BindFlags::default(),
            );
            let command = show_keymap_command(&table, false);
            let head = payload_of(&command);
            let mut expected = vec![terminal.to_owned()];
            expected.extend(exec.iter().map(|word| (*word).to_owned()));
            assert_eq!(head, expected, "wrong spelling for {terminal}");
        }
    }

    #[test]
    fn the_viewer_degrades_without_less_and_waits_on_the_terminal() {
        // The pager when one is there; plain output plus an explicit wait
        // on the terminal (not the pipe, which is at EOF by then) when not.
        assert!(
            KEYMAP_SCRIPT.contains("scoot msg binds"),
            "the script must show the live keymap: {KEYMAP_SCRIPT}"
        );
        assert!(
            KEYMAP_SCRIPT.contains("command -v less"),
            "the script must probe for the pager: {KEYMAP_SCRIPT}"
        );
        assert!(
            KEYMAP_SCRIPT.contains("less"),
            "the script must page when it can: {KEYMAP_SCRIPT}"
        );
        assert!(
            KEYMAP_SCRIPT.contains("read _ < /dev/tty"),
            "the fallback must wait on the terminal: {KEYMAP_SCRIPT}"
        );
    }

    #[test]
    fn super_shift_slash_still_opens_the_keymap() {
        // The default chord survives the move out of core: still bound, to
        // the same dedicated action the shell now expands.
        assert_eq!(
            Keybindings::default().match_key(Keysym::slash, SUPER_SHIFT),
            Some((Bound::Action(Action::ShowKeymap), BindFlags::default()))
        );
    }

    #[test]
    fn super_return_reports_its_spawn_argv() {
        assert_eq!(
            super_return_terminal(&Keybindings::default()),
            Some(vec!["foot".to_owned()])
        );
        let mut table = Keybindings::default();
        table.insert(
            SUPER,
            Keysym::Return,
            Bound::Action(Action::CloseFocused),
            BindFlags::default(),
        );
        assert_eq!(super_return_terminal(&table), None);
    }

    #[test]
    fn path_probe_finds_ls_and_misses_a_name_that_cannot_exist() {
        assert!(program_on_path("ls"), "ls must resolve on PATH");
        assert!(
            !program_on_path("scoot-no-such-program-f479"),
            "a missing program must not resolve"
        );
    }
}
