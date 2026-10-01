//! Running the commands a binding names (`{ exec = [...] }`): started,
//! bounded, and reaped, with no shell, no signal handler and no thread.
//!
//! - **Never through a shell.** The command line is an argument vector
//!   and is run as one; a user who wants a shell writes `sh -c` and owns
//!   the quoting.
//! - **Nothing of the bar's leaks into the child.** Its stdin, stdout and
//!   stderr are `/dev/null`; every fd the bar holds (the Wayland socket,
//!   the control socket and its lock, the shm memfds, the clock's timerfd,
//!   font and zone files) is `CLOEXEC`, which is what `std` sets on
//!   everything it opens and what the crate's own `rustix` calls ask for.
//!   `tests/pointer.rs::a_launched_command_holds_none_of_the_bars_descriptors`
//!   lists the fds of a command launched by a running bar, so a future fd
//!   opened without it fails a test. (`WAYLAND_SOCKET`, the
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
//!   A launched command otherwise lives as long as it likes: the bar does
//!   not supervise it.

use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags};
use rustix::process::{Pid, PidfdFlags, pidfd_open};

#[cfg(test)]
mod tests;

/// Launched commands running at once, at most.
pub const MAX_CHILDREN: usize = 8;

/// How often a child with no pidfd is looked at.
const FALLBACK_POLL: Duration = Duration::from_millis(250);

#[derive(Debug)]
struct Live {
    child: Child,
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
            return Err(format!(
                "not running `{program}`: {MAX_CHILDREN} launched commands are still running"
            ));
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
        self.live.push(Live { child, pidfd });
        Ok(())
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
