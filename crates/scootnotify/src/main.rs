//! `scootnotify`: the notification daemon.
//!
//! It owns `org.freedesktop.Notifications`, answers the four methods,
//! emits the signals, holds the live set with ids, replace, expiry, caps
//! and rate limits, draws the card stack on an on-demand overlay surface,
//! and serves `msg` on its control socket (the plan is
//! `dev/design/scootnotify.md`).
//!
//! No `unsafe` here. Nothing here links beyond what `std` links.

#![forbid(unsafe_code)]

// Tests pinning allocations count heap allocations through this
// (scootbg_mem::count), forwarding everything else to the system
// allocator. Test-only, Linux-only like the crate it comes from: the
// shipped binary uses the default.
#[cfg(all(test, target_os = "linux"))]
#[global_allocator]
static COUNTING_ALLOC: scootbg_mem::CountingAlloc = scootbg_mem::CountingAlloc;

use std::borrow::Cow;
use std::io::Write as _;
use std::time::{Duration, Instant};

use scoot_rt::print::{print, warn};
use scootnotify::control::client;
use scootnotify::control::paths;
use scootnotify::control::protocol::{EventKind, ReloadScope, Request};
use scootnotify::{cli, config, daemon};

fn main() {
    let code = run(&std::env::args().collect::<Vec<_>>());
    std::process::exit(code);
}

fn run(argv: &[String]) -> i32 {
    let args = match cli::parse_args(argv) {
        Ok(args) => args,
        Err(error) => {
            warn(format_args!("scootnotify: {error}"));
            return 2;
        }
    };
    if args.help {
        return match print(cli::HELP) {
            Ok(()) => 0,
            Err(_) => 1,
        };
    }
    if args.version {
        return match print(concat!("scootnotify ", env!("CARGO_PKG_VERSION"), "\n")) {
            Ok(()) => 0,
            Err(_) => 1,
        };
    }
    match args.command {
        cli::Command::Daemon => {
            if args.check {
                cmd_check(args.config.as_deref())
            } else {
                daemon::serve(args.config.as_deref(), args.font.as_deref())
            }
        }
        cli::Command::Msg(msg) => run_msg(msg),
    }
}

/// `daemon --check`: validates the config file and exits, without a bus
/// (0 valid, 1 refused). A missing file is the defaults, and valid.
fn cmd_check(config: Option<&str>) -> i32 {
    let path = config
        .map(std::path::PathBuf::from)
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
                "scootnotify: cannot read {}: {error}",
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
                warn(format_args!("scootnotify: {error}"));
                1
            }
        },
    }
}

/// `msg`: one request to the running daemon, one reply. `list`, `reload`
/// and `version` print the reply; `dismiss` and `invoke` print nothing on
/// success, as the bar's silent commands do; `subscribe` prints what the
/// daemon sends until it closes.
fn run_msg(msg: cli::Msg) -> i32 {
    match msg {
        cli::Msg::List { json: true } => send(&Request::List, true),
        cli::Msg::List { json: false } => list_human(),
        cli::Msg::Dismiss { id, all } => send(&Request::Dismiss { id, all }, false),
        cli::Msg::Invoke { id, action } => send(
            &Request::Invoke {
                id,
                action: Cow::Owned(action),
            },
            false,
        ),
        cli::Msg::Reload { look: false } => send(&Request::Reload { scope: None }, true),
        cli::Msg::Reload { look: true } => send(
            &Request::Reload {
                scope: Some(ReloadScope::Look),
            },
            true,
        ),
        cli::Msg::Subscribe { events } => subscribe(&Request::Subscribe { events }),
        cli::Msg::Wait {
            app,
            summary,
            timeout_secs,
        } => wait(app.as_deref(), summary.as_deref(), timeout_secs),
        cli::Msg::Version => send(&Request::Version, true),
    }
}

fn send(request: &Request<'_>, echoes: bool) -> i32 {
    match client::send(request) {
        Ok(_) if !echoes => 0,
        Ok(reply) => match print(&reply) {
            Ok(()) => 0,
            Err(_) => 1,
        },
        Err(error) => {
            warn(format_args!("scootnotify: {error}"));
            1
        }
    }
}

/// `list` for a human: one line per live card. The stable view is
/// `--json`; this is ink for a terminal.
fn list_human() -> i32 {
    let reply = match client::send(&Request::List) {
        Ok(reply) => reply,
        Err(error) => {
            warn(format_args!("scootnotify: {error}"));
            return 1;
        }
    };
    let parsed: serde_json::Value = match serde_json::from_str(&reply) {
        Ok(parsed) => parsed,
        Err(error) => {
            warn(format_args!(
                "scootnotify: unreadable reply from the daemon: {error}"
            ));
            return 1;
        }
    };
    let Some(cards) = parsed
        .get("notifications")
        .and_then(|cards| cards.as_array())
    else {
        warn(format_args!(
            "scootnotify: unreadable reply from the daemon: no `notifications`"
        ));
        return 1;
    };
    let mut out = String::new();
    for card in cards {
        // `list` speaks protocol 1, where every key below is always
        // present; a missing one is a loud error, not a panic and not a
        // blank line.
        let (Some(id), Some(urgency), Some(app), Some(summary)) = (
            card.get("id").and_then(|id| id.as_u64()),
            card.get("urgency").and_then(|urgency| urgency.as_str()),
            card.get("app_name").and_then(|app| app.as_str()),
            card.get("summary").and_then(|summary| summary.as_str()),
        ) else {
            warn(format_args!(
                "scootnotify: unreadable reply from the daemon: a card is misshapen"
            ));
            return 1;
        };
        out.push_str(&format!("{id} [{urgency}] {app}: {summary}\n"));
    }
    match print(out.as_str()) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// `subscribe`: prints the daemon's lines as they come, until it closes the
/// connection between two lines (exit status 0, the same quiet-EPIPE
/// contract the bar's `msg subscribe` keeps) or stdout closes first. A
/// `dropped` line, or a connection that ends inside a line (the daemon
/// dropped the subscriber mid-write, which discards that line), is exit
/// status 1. Status 0 is not "the daemon exited": a subscriber dropped
/// with a full socket ends the same way, so a caller resubscribes and
/// lists after any end.
fn subscribe(request: &Request<'_>) -> i32 {
    let mut stdout_closed = false;
    let result = client::stream(request, |line| {
        if print(line).is_err() {
            stdout_closed = true;
            return false;
        }
        true
    });
    match result {
        Ok(()) if stdout_closed => 1,
        Ok(()) => 0,
        Err(error) => {
            warn(format_args!("scootnotify: {error}"));
            1
        }
    }
}

/// `wait`: subscribes, reads the baseline `list`, and blocks for the
/// first notification matching every filter given. Prints its JSON and
/// exits 0; on the timeout, exits 1. Subscribing first means nothing
/// arriving between the baseline and the watch is missed.
fn wait(app: Option<&str>, summary: Option<&str>, timeout_secs: u64) -> i32 {
    let paths = match paths::from_env("scootnotify") {
        Ok(paths) => paths,
        Err(error) => {
            warn(format_args!("scootnotify: {error}"));
            return 1;
        }
    };
    let request = Request::Subscribe {
        events: vec![EventKind::Notify],
    };
    let mut subscription = match client::subscribe_to(&paths, &request) {
        Ok(subscription) => subscription,
        Err(error) => {
            warn(format_args!("scootnotify: {error}"));
            return 1;
        }
    };
    let baseline = match client::send_to(&paths, &Request::List) {
        Ok(baseline) => baseline,
        Err(error) => {
            warn(format_args!("scootnotify: {error}"));
            return 1;
        }
    };
    if let Some(found) = first_match(&baseline, app, summary) {
        return match print(found.as_str()) {
            Ok(()) => 0,
            Err(_) => 1,
        };
    }
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            warn(format_args!(
                "scootnotify: timed out after {timeout_secs}s with no matching notification"
            ));
            return 1;
        }
        match subscription.read_line(Some(left)) {
            Err(error) => {
                warn(format_args!("scootnotify: {error}"));
                return 1;
            }
            // With time left, a socket timeout cannot have fired: the
            // daemon closed the subscription, and events may be missed.
            Ok(None) => {
                warn(format_args!(
                    "scootnotify: the daemon closed the subscription before anything matched"
                ));
                return 1;
            }
            Ok(Some(line)) => {
                if line.as_bytes() == scootnotify::control::protocol::DROPPED {
                    warn(format_args!(
                        "scootnotify: the daemon dropped this subscription: events may have been \
                         missed: subscribe again, then list"
                    ));
                    return 1;
                }
                if let Some(found) = first_match(&line, app, summary) {
                    return match print(found.as_str()) {
                        Ok(()) => 0,
                        Err(_) => 1,
                    };
                }
            }
        }
    }
}

/// The first notification in `text` matching every filter, as compact
/// JSON with its newline. `text` is a `list` reply (whose
/// `notifications` array is searched) or one event line (whose
/// `notification` is checked). Anything unparseable, or any other event,
/// matches nothing: a half-written line is never a match.
fn first_match(text: &str, app: Option<&str>, summary: Option<&str>) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    if parsed.get("type") == Some(&serde_json::Value::from("notify")) {
        let note = parsed.get("notification")?;
        return matches(note, app, summary).then(|| {
            let mut found = serde_json::to_string(note).unwrap_or_default();
            found.push('\n');
            found
        });
    }
    let cards = parsed.get("notifications")?.as_array()?;
    for note in cards {
        if matches(note, app, summary) {
            let mut found = serde_json::to_string(note).unwrap_or_default();
            found.push('\n');
            return Some(found);
        }
    }
    None
}

/// Whether one notification object matches every filter given: `--app`
/// names its app exactly, `--summary-contains` is a substring of its
/// summary. No filters match anything.
fn matches(note: &serde_json::Value, app: Option<&str>, summary: Option<&str>) -> bool {
    if let Some(app) = app
        && note.get("app_name") != Some(&serde_json::Value::from(app))
    {
        return false;
    }
    if let Some(needle) = summary {
        let summary = note.get("summary").and_then(|summary| summary.as_str());
        if summary.is_none_or(|summary| !summary.contains(needle)) {
            return false;
        }
    }
    true
}
