//! `scootnotify`: the notification daemon.
//!
//! This ticket is the D-Bus server and the model, headless: it owns
//! `org.freedesktop.Notifications`, answers the four methods, emits the
//! signals, and holds the live set with ids, replace, expiry, caps and
//! rate limits. Popups, the control socket and Nix arrive in later
//! tickets (the plan is `dev/design/scootnotify.md`).
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

use std::io::Write as _;

use scoot_rt::print::{print, warn};
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
    if args.check {
        return cmd_check(args.config.as_deref());
    }
    daemon::serve(args.config.as_deref())
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
