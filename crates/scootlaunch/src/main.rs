//! `scootlaunch`: a keyboard-first application launcher for Wayland.
//!
//! This ticket (`docs/scootbar/backlog/launch-desktop-index.md`) builds
//! only the desktop entry index and the headless agent interface: data
//! dirs and ids with precedence and `Hidden` masking, the format and its
//! escapes, localized keys, `OnlyShowIn`/`NotShowIn` and `TryExec`,
//! actions, the Exec quoting and field codes, and the cache keyed by
//! canonical path, inode and mtime — plus `scootlaunch --list [--all]
//! [--lenient] [--json]`. Ranking, the panel, launching and the dmenu
//! mode are later tickets. The plan is `dev/design/scootlaunch.md`.
//!
//! No `unsafe` here. Nothing here links beyond what `std` links.

#![forbid(unsafe_code)]

mod cli;
mod index;

use std::io::Write as _;

use scoot_rt::print::{print, warn};

fn main() {
    let code = run(&std::env::args().collect::<Vec<_>>());
    std::process::exit(code);
}

fn run(argv: &[String]) -> i32 {
    let args = match cli::parse_args(argv) {
        Ok(args) => args,
        Err(error) => {
            warn(format_args!("scootlaunch: {error}"));
            return 2;
        }
    };
    if args.help {
        match print(cli::HELP) {
            Ok(()) => 0,
            Err(_) => 1,
        }
    } else if args.version {
        match print(concat!("scootlaunch ", env!("CARGO_PKG_VERSION"), "\n")) {
            Ok(()) => 0,
            Err(_) => 1,
        }
    } else {
        cmd_list(&args)
    }
}

/// `--list`: every visible entry, or with `--all` the hidden ones too
/// with their reason. Human-readable lines by default, the agent JSON
/// schema with `--json` (`dev/design/scootlaunch.md`, The agent
/// interface).
fn cmd_list(args: &cli::Args) -> i32 {
    let env = index::Env::from_env();
    let opts = index::BuildOpts {
        use_cache: true,
        lenient_exec: args.lenient,
    };
    let built = index::build(&env, &opts);
    for note in &built.notes {
        warn(format_args!("scootlaunch: {note}"));
    }
    let mut out = String::new();
    if args.json {
        let entries: Vec<index::JsonEntry> = built
            .entries
            .iter()
            .filter(|entry| args.all || entry.hidden.is_none())
            .map(index::JsonEntry::from_entry)
            .collect();
        match serde_json::to_string(&entries) {
            Ok(json) => {
                out.push_str(&json);
                out.push('\n');
            }
            Err(error) => {
                warn(format_args!("scootlaunch: cannot encode --json: {error}"));
                return 1;
            }
        }
    } else {
        for entry in &built.entries {
            if !args.all && entry.hidden.is_some() {
                continue;
            }
            out.push_str(&entry.id);
            out.push('\t');
            out.push_str(&entry.name);
            if let Some(reason) = &entry.hidden {
                out.push_str(" [hidden: ");
                out.push_str(reason.as_str());
                out.push(']');
            }
            out.push('\n');
        }
    }
    // One write: a reader that went away (`--list | head -c0`) is a
    // quiet success, as for any Unix tool.
    match print(&out) {
        Ok(()) => 0,
        Err(error) => {
            let _ = std::io::stderr()
                .write_fmt(format_args!("scootlaunch: cannot write output: {error}\n"));
            1
        }
    }
}
