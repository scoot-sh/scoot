//! Running a rule's `exec`: started, bounded, and reaped, with no
//! shell, no signal handler and no thread.
//!
//! The bar's `spawn` discipline, narrowed to the notifier's bound:
//! - **Never through a shell.** The command line is an argument vector
//!   and is run as one; a user who wants a shell writes `sh -c` and owns
//!   the quoting.
//! - **Nothing the daemon opens leaks into the child.** Its stdin,
//!   stdout and stderr are `/dev/null`; every fd the daemon opens is
//!   `CLOEXEC`, which is what `std` sets on everything it opens and what
//!   the crate's own `rustix` calls ask for. The child leads its own
//!   process group, so a signal to the daemon's group does not reach
//!   what a rule launched.
//! - **The notification's fields ride in the environment** as
//!   `SCOOTNOTIFY_*` (`ID`, `APP`, `SUMMARY`, `BODY`, `URGENCY`,
//!   `CATEGORY`, `DESKTOP_ENTRY`), so a sound hook or a do-something
//!   script reads them without parsing arguments. Values are the
//!   already-bounded fields (the summary at most 1 KiB, the body at most
//!   8 KiB), never pixels.
//! - **Reaped the moment it exits, at no idle cost.** Each child's pidfd
//!   (`pidfd_open`, Linux 5.3) is polled by the loop and becomes readable
//!   when the child exits; [`Spawner::reap`] then collects it, so no
//!   zombie outlives its exit by more than a turn of the loop, with no
//!   `SIGCHLD` handler (which `#![forbid(unsafe_code)]` rules out), no
//!   timer and no thread. On a kernel without pidfds the children are
//!   collected by [`Spawner::reap`] on a short timeout while any is
//!   running ([`Spawner::poll_timeout`]), and never otherwise.
//! - **Bounded.** At most [`MAX_CHILDREN`] rule children run at once;
//!   one past it is refused with a message, not queued, so a rule that
//!   matches a flood (or a hook that hangs) cannot fill the process
//!   table. A child otherwise lives as long as it likes: the daemon does
//!   not supervise it.

use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags};
use rustix::process::{Pid, PidfdFlags, pidfd_open};

/// Rule children running at once, at most: the design's bound (the
/// bar's own is 8; a notification hook is smaller and more
/// flood-exposed, so 4).
pub const MAX_CHILDREN: usize = 4;

/// How often a child with no pidfd is looked at.
const FALLBACK_POLL: Duration = Duration::from_millis(250);

/// One `exec` a rule fired: the argv with the notification's fields.
/// Built by the store (pure state), spawned by the daemon (I/O).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecRequest {
    /// The argv: the program, then its arguments.
    pub argv: Vec<String>,
    /// The notification's id, for `SCOOTNOTIFY_ID`.
    pub id: u32,
    pub app_name: String,
    pub summary: String,
    pub body: String,
    /// `low`, `normal` or `critical`, for `SCOOTNOTIFY_URGENCY`.
    pub urgency: &'static str,
    pub category: String,
    pub desktop_entry: String,
}

impl ExecRequest {
    /// The environment the child reads: the notification's fields under
    /// `SCOOTNOTIFY_*`.
    fn env(&self) -> [(&'static str, &str); 7] {
        [
            ("SCOOTNOTIFY_APP", self.app_name.as_str()),
            ("SCOOTNOTIFY_SUMMARY", self.summary.as_str()),
            ("SCOOTNOTIFY_BODY", self.body.as_str()),
            ("SCOOTNOTIFY_URGENCY", self.urgency),
            ("SCOOTNOTIFY_CATEGORY", self.category.as_str()),
            ("SCOOTNOTIFY_DESKTOP_ENTRY", self.desktop_entry.as_str()),
            ("SCOOTNOTIFY_ID", ""),
        ]
    }
}

#[derive(Debug)]
struct Live {
    child: Child,
    /// Readable when the child exits; `None` on a kernel without pidfds.
    pidfd: Option<OwnedFd>,
}

#[derive(Debug, Default)]
pub struct Spawner {
    live: Vec<Live>,
    /// Whether to ask for pidfds (off only in tests, to reach the
    /// fallback).
    pidfds: bool,
}

impl Spawner {
    /// Launches `request`'s argv with its environment. `Err` says why
    /// not, for a line on stderr.
    pub fn spawn(&mut self, request: &ExecRequest) -> Result<(), String> {
        let Some((program, args)) = request.argv.split_first() else {
            return Err(String::from("a rule exec is empty"));
        };
        // Children that already exited free their slots first.
        self.reap();
        if self.live.len() >= MAX_CHILDREN {
            return Err(format!(
                "not running `{program}`: {MAX_CHILDREN} rule children are still running"
            ));
        }
        let id = request.id.to_string();
        let env = request.env();
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        for (key, value) in &env {
            if *key == "SCOOTNOTIFY_ID" {
                command.env(key, &id);
            } else {
                command.env(key, value);
            }
        }
        let child = command
            .spawn()
            .map_err(|error| format!("cannot run `{program}`: {error}"))?;
        let pidfd = if self.pidfds {
            pidfd_open(Pid::from_child(&child), PidfdFlags::empty()).ok()
        } else {
            None
        };
        self.live.push(Live { child, pidfd });
        Ok(())
    }

    /// Adds every child's pidfd to `fds` from index `*len` on (up to
    /// `fds`' end), each polled for exit. The loop calls
    /// [`Spawner::reap`] when any of them is ready.
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

    /// How long the loop may sleep while a child with no pidfd runs;
    /// `None` (sleep as long as it likes) otherwise, which is always, on
    /// a kernel with pidfds.
    pub fn poll_timeout(&self) -> Option<Duration> {
        self.live
            .iter()
            .any(|live| live.pidfd.is_none())
            .then_some(FALLBACK_POLL)
    }

    /// How many rule children are running.
    #[cfg(test)]
    pub fn running(&self) -> usize {
        self.live.len()
    }

    /// Kills and waits every running child: tests only, so a flood
    /// test's sleeps never outlive it. Slots free only through
    /// [`Spawner::reap`]: collecting the waited is the spawner's own
    /// job, and the flood tests pin it.
    #[cfg(test)]
    pub fn kill_all(&mut self) {
        for live in &mut self.live {
            let _ = live.child.kill();
            let _ = live.child.wait();
        }
    }
}

#[cfg(test)]
mod tests;
