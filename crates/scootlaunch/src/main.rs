//! `scootlaunch`: a keyboard-first application launcher for Wayland.
//!
//! The desktop entry index and the headless agent interface: data
//! dirs and ids with precedence and `Hidden` masking, the format and
//! its escapes, localized keys, `OnlyShowIn`/`NotShowIn` and
//! `TryExec`, actions, the Exec quoting and field codes, and the
//! cache keyed by canonical path, inode and mtime — plus fuzzy
//! matching with frecency (`--list` and `--query`, each with
//! `[--all] [--lenient] [--json]`), the overlay exclusive-keyboard
//! panel (bare `scootlaunch`, with the entry and the virtualized list,
//! the single-instance toggle and the look layer), launching (the
//! activation token, the environment, the systemd scope, the detached
//! fallback, `DBusActivatable`), the dmenu mode (stdin lines in, the
//! pick on stdout, `--dmenu`) and run mode (executables on `$PATH`,
//! `--run`). The plan is `dev/design/scootlaunch.md`.
//!
//! No `unsafe` here. The keyboard links libxkbcommon through `scootui`
//! (decided in `dev/design/shell-apps.md`); nothing else here links
//! beyond what `std` links.

#![forbid(unsafe_code)]

mod cli;
mod config;
mod dmenu;
mod index;
mod look;
mod panel;
mod rank;
mod run;
mod socket;
mod spawn;
mod ui;

// Tests pinning the warm-query path count heap allocations through
// this (scootbg_mem::count), forwarding everything else to the
// system allocator. Test-only, Linux-only like the crate it comes
// from: the shipped binary uses the default.
#[cfg(all(test, target_os = "linux"))]
#[global_allocator]
static COUNTING_ALLOC: scootbg_mem::CountingAlloc = scootbg_mem::CountingAlloc;

use std::io::Write as _;
use std::path::PathBuf;

use scoot_rt::print::{print, warn};

fn main() {
    let code = run(&std::env::args().collect::<Vec<_>>());
    std::process::exit(code);
}

fn run(argv: &[String]) -> i32 {
    // The detached spawn's helper re-exec (no flag, no stdout
    // contract: stdin carries the spec, the marker names it).
    if spawn::is_detach_helper() {
        return spawn::detach_main();
    }
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
    } else if args.query.is_some() {
        cmd_query(&args)
    } else if args.list {
        cmd_list(&args)
    } else if args.check {
        cmd_check(args.config.as_deref())
    } else if args.dmenu {
        cmd_dmenu(&args)
    } else if args.run {
        panel::open(panel::Options {
            font: args.font.map(PathBuf::from),
            config: args.config.map(PathBuf::from),
            lenient: args.lenient,
            mode: panel::Mode::Run {
                colors: args.colors,
            },
        })
    } else {
        panel::open(panel::Options {
            font: args.font.map(PathBuf::from),
            config: args.config.map(PathBuf::from),
            lenient: args.lenient,
            mode: panel::Mode::Apps {
                colors: args.colors,
            },
        })
    }
}

/// `--dmenu [flags]`: stdin lines through the panel, the pick on
/// stdout (exit 0), Escape exits 1. A pick prints and launches
/// nothing. `--prompt-only` never reads stdin (an input field with no
/// list); `--no-run-if-empty` exits 1 without opening on empty stdin.
fn cmd_dmenu(args: &cli::Args) -> i32 {
    // `--prompt-only` does not read anything from stdin (fuzzel's
    // meaning): the panel opens on no lines.
    let lines = if args.prompt_only.is_some() {
        Vec::new()
    } else {
        dmenu::read_stdin()
    };
    if args.no_run_if_empty && lines.is_empty() {
        return 1;
    }
    let prompt = args
        .prompt_only
        .clone()
        .or_else(|| args.prompt.clone())
        .unwrap_or_else(|| "> ".to_string());
    let colors = cli::ColorOverrides {
        background: args.colors.background,
        text: args.colors.text,
        border: args.colors.border,
        selection: args.colors.selection,
        selection_text: args.colors.selection_text,
        r#match: args.colors.r#match,
        prompt: args.colors.prompt,
    };
    panel::open(panel::Options {
        font: args.font.clone().map(PathBuf::from),
        config: args.config.clone().map(PathBuf::from),
        lenient: args.lenient,
        mode: panel::Mode::Dmenu(panel::DmenuConfig {
            prompt,
            prompt_only: args.prompt_only.is_some(),
            password: args.password,
            only_match: args.only_match,
            index: args.index,
            placeholder: args.placeholder.clone().unwrap_or_default(),
            lines,
            colors,
        }),
    })
}

/// `--check [--config PATH]`: validates the panel's config file and
/// exits, without a compositor (0 valid, 1 refused, as the bar's and the
/// notification daemon's `--check`). A missing file is the defaults, and
/// valid.
fn cmd_check(config: Option<&str>) -> i32 {
    let path = config
        .map(PathBuf::from)
        .unwrap_or_else(config::default_path);
    match std::fs::read(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let _ = std::io::stdout().write_fmt(format_args!(
                "{}: no config, built-in defaults apply\n",
                path.display()
            ));
            0
        }
        Err(error) => {
            warn(format_args!(
                "scootlaunch: cannot read {}: {error}",
                path.display()
            ));
            1
        }
        Ok(bytes) => match config::parse(&bytes) {
            Ok(_) => {
                let _ = std::io::stdout().write_fmt(format_args!("{}: valid\n", path.display()));
                0
            }
            Err(error) => {
                warn(format_args!("scootlaunch: {error}"));
                1
            }
        },
    }
}

/// `--query TEXT`: entries ranked for TEXT the way the UI ranks
/// them — fuzzy match plus frecency — each entry once at its best
/// row's position (an action row still prints its entry). An empty
/// TEXT orders by frecency, then name. Human-readable lines by
/// default, the agent JSON schema with `--json`, in ranked order.
fn cmd_query(args: &cli::Args) -> i32 {
    let query = args.query.as_deref().unwrap_or("");
    let env = index::Env::from_env();
    let opts = index::BuildOpts {
        use_cache: true,
        lenient_exec: args.lenient,
    };
    let built = index::build(&env, &opts);
    for note in &built.notes {
        warn(format_args!("scootlaunch: {note}"));
    }
    let history = rank::frecency::load(env.xdg_state_home.as_deref());
    let rows = rank::build_rows(&built.entries);
    let mut ranker = rank::Ranker::new();
    let now = rank::frecency::now_secs();
    let order = ranker.rank(query, &rows, &history, now).to_vec();
    let ranked = rank::ranked_entries(&built.entries, &rows, &order, args.all);
    let mut out = String::new();
    if args.json {
        let entries: Vec<index::JsonEntry> = ranked
            .iter()
            .map(|entry| index::JsonEntry::from_entry(entry))
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
        for entry in &ranked {
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
    // One write: a reader that went away (`--query x | head -c0`) is
    // a quiet success, as for any Unix tool.
    match print(&out) {
        Ok(()) => 0,
        Err(error) => {
            let _ = std::io::stderr()
                .write_fmt(format_args!("scootlaunch: cannot write output: {error}\n"));
            1
        }
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
