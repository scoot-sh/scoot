//! Running the commands a binding names (`{ exec = [...] }`): started,
//! bounded, and reaped, with no shell, no signal handler and no thread.
//!
//! - **Never through a shell.** The command line is an argument vector
//!   and is run as one; a user who wants a shell writes `sh -c` and owns
//!   the quoting.
//! - **Nothing the bar opens leaks into the child.** Its stdin, stdout and
//!   stderr are `/dev/null`; every fd the bar opens (the Wayland socket,
//!   the control socket and its lock, the shm memfds, the clock's timerfd,
//!   font and zone files) is `CLOEXEC`, which is what `std` sets on
//!   everything it opens and what the crate's own `rustix` calls ask for.
//!   What a launcher left open in the bar at its start (a descriptor
//!   without `CLOEXEC` that the bar was exec'd with) is not the bar's, and
//!   reaches every child as it would any process's.
//!   `tests/pointer.rs::a_launched_command_holds_none_of_the_bars_descriptors`
//!   checks that the running bar holds no descriptor without `CLOEXEC`
//!   beyond what its launcher handed it, and lists the fds of a command it
//!   launched, so a future fd opened without it fails a test.
//!   (`WAYLAND_SOCKET`, the
//!   fd number a compositor may have started the bar with, is removed from
//!   the environment by `wayland-client` when it takes the socket.) The
//!   child leads its own process group, so a signal to the bar's group
//!   does not reach what the bar launched.
//! - **Reaped the moment it exits, at no idle cost.** Each child's pidfd
//!   (`pidfd_open`, Linux 5.3) is polled by the loop and becomes readable
//!   when the child exits; [`Spawner::reap`] then collects it, so no
//!   zombie outlives its exit by more than a turn of the loop, with no
//!   `SIGCHLD` handler (which `#![forbid(unsafe_code)]` rules out, and
//!   whose blocked mask would leak into every child besides), no timer and
//!   no thread. On a kernel without pidfds the children are collected by
//!   [`Spawner::reap`] on a short timeout while any is running
//!   ([`Spawner::poll_timeout`]), and never otherwise.
//! - **Bounded.** At most [`MAX_CHILDREN`] launched commands run at once;
//!   one past it is refused with a message, not queued, so a flood of
//!   clicks (or a bound command that hangs) cannot fill the process table.
//!   The message names what holds the table (program basename, pid, how
//!   long it has run, oldest first) so the user knows what to stop by
//!   hand: the bar never signals a process it launched. A launched
//!   command otherwise lives as long as it likes: the bar does not
//!   supervise it. (A wait-per-spawn queue would be small here, `reap` is
//!   already the one place slots free, but it is declined: a queued
//!   command would fire arbitrarily late.)

use std::fmt::Write as _;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags};
use rustix::process::{Pid, PidfdFlags, pidfd_open};

#[cfg(test)]
mod tests;

/// Launched commands running at once, at most.
pub const MAX_CHILDREN: usize = 8;

/// How often a child with no pidfd is looked at.
const FALLBACK_POLL: Duration = Duration::from_millis(250);

/// Bytes of a program's name kept for the refusal message.
const NAME_MAX: usize = 24;

/// A program's basename, bounded and printable, kept inline so launching
/// allocates nothing for it.
#[derive(Debug, Clone, Copy)]
struct Name {
    buf: [u8; NAME_MAX],
    len: u8,
    /// The name was longer than [`NAME_MAX`] and was cut.
    cut: bool,
}

impl Name {
    /// The part of `program` after its last `/`, control characters shown
    /// as `?`, cut at a character boundary.
    fn of(program: &str) -> Self {
        let base = program.rsplit('/').next().unwrap_or(program);
        let mut name = Self {
            buf: [0; NAME_MAX],
            len: 0,
            cut: false,
        };
        let mut at = 0;
        for c in base.chars() {
            let c = if c.is_control() { '?' } else { c };
            let end = at + c.len_utf8();
            if end > NAME_MAX {
                name.cut = true;
                break;
            }
            c.encode_utf8(&mut name.buf[at..end]);
            at = end;
        }
        // `at <= NAME_MAX = 24`, which fits in a `u8`.
        name.len = at as u8;
        name
    }

    fn as_str(&self) -> &str {
        // Only whole characters were written, so this never fails.
        std::str::from_utf8(&self.buf[..usize::from(self.len)]).unwrap_or("?")
    }
}

impl std::fmt::Display for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())?;
        if self.cut {
            f.write_str("\u{2026}")?;
        } else if self.len == 0 {
            f.write_str("?")?;
        }
        Ok(())
    }
}

/// `elapsed` for a person: `7s`, `2m 05s`, `3h 02m`.
fn age(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, secs % 3600 / 60),
    }
}

#[derive(Debug)]
struct Live {
    child: Child,
    name: Name,
    /// Monotonic, for the refusal's ages.
    started: Instant,
    /// Readable when the child exits; `None` on a kernel without pidfds.
    pidfd: Option<OwnedFd>,
}

#[derive(Debug)]
pub struct Spawner {
    live: Vec<Live>,
    /// Whether to ask for pidfds (off only in tests, to reach the
    /// fallback).
    pidfds: bool,
}

impl Default for Spawner {
    fn default() -> Self {
        Self {
            live: Vec::with_capacity(MAX_CHILDREN),
            pidfds: true,
        }
    }
}

impl Spawner {
    /// Launches `argv` (the program, then its arguments). `Err` says why
    /// not, for a line on stderr.
    pub fn spawn(&mut self, argv: &[String]) -> Result<(), String> {
        let Some((program, args)) = argv.split_first() else {
            return Err("an empty command".to_owned());
        };
        // Children that already exited free their slots first.
        self.reap();
        if self.live.len() >= MAX_CHILDREN {
            // A child may have exited since: look once more, so the
            // message never names one that is gone.
            self.reap();
            if self.live.len() >= MAX_CHILDREN {
                return Err(self.refusal(program, Instant::now()));
            }
        }
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        let child = command
            .spawn()
            .map_err(|error| format!("cannot run `{program}`: {error}"))?;
        let pidfd = if self.pidfds {
            pidfd_open(Pid::from_child(&child), PidfdFlags::empty()).ok()
        } else {
            None
        };
        self.live.push(Live {
            child,
            name: Name::of(program),
            started: Instant::now(),
            pidfd,
        });
        Ok(())
    }

    /// Why `program` was not run, naming what holds the table (oldest
    /// first, the order they were launched in) as of `now`. Built only
    /// when refusing.
    fn refusal(&self, program: &str, now: Instant) -> String {
        let mut text = format!(
            "not running `{}`: {} launched commands are still running (stop one by hand to free a slot):",
            Name::of(program),
            self.live.len()
        );
        for (index, live) in self.live.iter().enumerate() {
            let _ = write!(
                text,
                "{} `{}` pid {} for {}",
                if index == 0 { "" } else { "," },
                live.name,
                live.child.id(),
                age(now.saturating_duration_since(live.started)),
            );
        }
        text
    }

    /// Adds every child's pidfd to `fds` from index `*len` on (up to
    /// `fds`' end), each polled for exit. The loop calls [`Spawner::reap`]
    /// when any of them is ready.
    pub fn sources<'fd>(&'fd self, fds: &mut [PollFd<'fd>], len: &mut usize) {
        for live in &self.live {
            let Some(pidfd) = &live.pidfd else {
                continue;
            };
            let Some(slot) = fds.get_mut(*len) else {
                return;
            };
            *slot = PollFd::from_borrowed_fd(pidfd.as_fd(), PollFlags::IN);
            *len += 1;
        }
    }

    /// Collects every child that has exited.
    pub fn reap(&mut self) {
        self.live
            .retain_mut(|live| matches!(live.child.try_wait(), Ok(None)));
    }

    /// How long the loop may sleep while a child with no pidfd runs; `None`
    /// (sleep as long as it likes) otherwise, which is always, on a kernel
    /// with pidfds.
    pub fn poll_timeout(&self) -> Option<Duration> {
        self.live
            .iter()
            .any(|live| live.pidfd.is_none())
            .then_some(FALLBACK_POLL)
    }

    /// How many launched commands are running.
    #[cfg(test)]
    pub fn running(&self) -> usize {
        self.live.len()
    }
}
