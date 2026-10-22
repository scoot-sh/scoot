//! `scoot-globalshortcuts-portal`: the GlobalShortcuts portal backend.
//!
//! It owns `org.freedesktop.impl.portal.desktop.scoot`, serves
//! `org.freedesktop.impl.portal.GlobalShortcuts` version 2, and proxies
//! registrations at the compositor's stage-1 IPC (`shortcut-register`,
//! `shortcuts`, the `global-shortcut` subscription). Assignment stays
//! where stage 1 put it: the user's config file.
//!
//! No `unsafe` here. Nothing here links beyond what `std` links.

#![forbid(unsafe_code)]

mod cli;
mod daemon;
mod ipc;
mod portal;

use scoot_rt::print::{print, warn};

fn main() {
    let code = run(&std::env::args().collect::<Vec<_>>());
    std::process::exit(code);
}

fn run(argv: &[String]) -> i32 {
    let args = match cli::parse_args(argv) {
        Ok(args) => args,
        Err(error) => {
            warn(format_args!("scoot-globalshortcuts-portal: {error}"));
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
        return match print(concat!(
            "scoot-globalshortcuts-portal ",
            env!("CARGO_PKG_VERSION"),
            "\n"
        )) {
            Ok(()) => 0,
            Err(_) => 1,
        };
    }
    match args.command {
        cli::Command::Daemon => daemon::serve(),
    }
}
