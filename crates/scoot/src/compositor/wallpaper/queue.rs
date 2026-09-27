//! Which `scootbg apply-config` runs, and when: one at a time, newest
//! section wins, a failure retried, a hung run given up on after a bound.
//!
//! Pure: no processes, no clock, no loop. [`State`](super::super::State)'s
//! glue (`wallpaper.rs`) spawns what [`Queue::next`] names, reports what
//! happened through [`Queue::started`], [`Queue::spawn_failed`],
//! [`Queue::reap`] and [`Queue::expire`], and arms one timer per run at
//! [`Queue::deadline`]. So every rule below is a unit test.
//!
//! # The rules
//!
//! - **One run at a time.** Two reloads in quick succession would otherwise
//!   start two `apply-config`s that race, and the older section could land
//!   last. A section submitted while a run is in flight waits, and only the
//!   newest waits: a third submission replaces the second.
//! - **Bounded.** A run is waited on for [`PATIENCE`] at most. An
//!   `apply-config` bounds itself (5 s to find a daemon, 30 s for its
//!   reply), except `{}` with no daemon, which writes the state file holding
//!   the display's lock and can sit in `fsync` on a hung disk for as long as
//!   the kernel does (see the ticket's F4). Past the bound the run is
//!   *abandoned*: still reaped and its exit still logged when it comes, but
//!   no longer blocking the next section.
//! - **A failure is not final** (N1). A run of the newest section that fails
//!   (any status but 0, or killed) keeps that section queued, *held*: it is
//!   not retried at once (the same run would likely fail the same way, and
//!   in a loop), but on the next *trigger*: a reload, or an abandoned run
//!   exiting (typically the hung `{}` finishing, which frees the lock the
//!   failed run was waiting on). A spawn that fails (a missing binary) is
//!   held the same way.
//! - **A late run is followed by the newest.** An abandoned run that exits
//!   was for an older section, or for this one: either way its change may
//!   have landed after a newer run's, so the newest section is run again.
//!   An unchanged section costs scootbg one fingerprint comparison, so this
//!   is cheap, and it only happens after a run was abandoned.
//! - **At most [`MAX_ABANDONED`] abandoned runs.** Each is a live process;
//!   a scootbg that hung every time would otherwise add one every
//!   [`PATIENCE`]. At the cap nothing new starts until one exits.

use std::ffi::OsString;
use std::time::{Duration, Instant};

/// How long a run is waited on before it is abandoned: `apply-config`'s own
/// bounds (5 s to reach a daemon, then 30 s for the reply) plus margin, so
/// a run that is merely slow is never abandoned.
pub const PATIENCE: Duration = Duration::from_secs(40);

/// The most abandoned runs kept (and so the most `apply-config` processes
/// alive at once, less the one in flight) before nothing new starts.
pub const MAX_ABANDONED: usize = 8;

/// One section to hand scootbg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submission {
    /// Increases with every [`Queue::submit`]; what "newest" means.
    pub seq: u64,
    pub command: OsString,
    pub json: String,
}

/// A running (or abandoned) `apply-config`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Run {
    pid: u32,
    seq: u64,
    started: Instant,
    /// What ran, for the log line at its end.
    command: OsString,
}

/// Whether the newest section still needs a run.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Want {
    /// No: it is running, or it ran, or nothing was ever submitted.
    #[default]
    Idle,
    /// Yes, as soon as nothing is in flight.
    Now,
    /// Yes, but its last run (or spawn) failed: on the next trigger.
    Held,
}

/// What `waitpid` said about a tracked pid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waited {
    /// Still running.
    Running,
    /// Exited, with this raw `waitpid` status.
    Exited(i32),
    /// Not ours any more (`ECHILD`): gone without a status.
    Gone,
}

/// A run's end, for the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Success,
    Code(i32),
    Signal(i32),
    /// Reaped elsewhere, status unknown.
    Unknown,
}

impl Exit {
    pub fn from_status(status: i32) -> Self {
        if libc::WIFEXITED(status) {
            match libc::WEXITSTATUS(status) {
                0 => Self::Success,
                code => Self::Code(code),
            }
        } else if libc::WIFSIGNALED(status) {
            Self::Signal(libc::WTERMSIG(status))
        } else {
            // `waitpid` without `WUNTRACED`/`WCONTINUED` reports only exits
            // and deaths by signal; anything else is not an end.
            Self::Unknown
        }
    }

    fn succeeded(self) -> bool {
        self == Self::Success
    }
}

/// An exited run, as [`Queue::reap`] reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ended {
    pub pid: u32,
    pub command: OsString,
    pub exit: Exit,
    /// Whether it had been abandoned (waited on past [`PATIENCE`]).
    pub abandoned: bool,
}

#[derive(Debug)]
pub struct Queue {
    newest: Option<Submission>,
    want: Want,
    running: Option<Run>,
    abandoned: Vec<Run>,
    next_seq: u64,
    /// [`PATIENCE`], except in tests of the glue's timer, which cannot wait
    /// 40 s.
    patience: Duration,
}

impl Default for Queue {
    fn default() -> Self {
        Self {
            newest: None,
            want: Want::Idle,
            running: None,
            abandoned: Vec::new(),
            next_seq: 0,
            patience: PATIENCE,
        }
    }
}

impl Queue {
    /// Queues a section as the newest, replacing any not yet started. A
    /// trigger in itself: whatever was held is superseded.
    pub fn submit(&mut self, command: OsString, json: String) {
        self.next_seq += 1;
        self.newest = Some(Submission {
            seq: self.next_seq,
            command,
            json,
        });
        self.want = Want::Now;
    }

    /// A reload that submits nothing new (the section is still absent, or
    /// is invalid): a held section is retried.
    pub fn trigger(&mut self) {
        if self.want == Want::Held {
            self.want = Want::Now;
        }
    }

    /// The section to start now, if any: the newest, when it needs a run,
    /// nothing is in flight, and fewer than [`MAX_ABANDONED`] runs are
    /// abandoned. The caller spawns it and reports [`Queue::started`] or
    /// [`Queue::spawn_failed`].
    pub fn next(&self) -> Option<&Submission> {
        if self.want != Want::Now || self.running.is_some() || self.abandoned.len() >= MAX_ABANDONED
        {
            return None;
        }
        self.newest.as_ref()
    }

    /// The section [`Queue::next`] named is running as `pid`.
    pub fn started(&mut self, pid: u32, now: Instant) {
        let Some(newest) = &self.newest else {
            return;
        };
        self.running = Some(Run {
            pid,
            seq: newest.seq,
            started: now,
            command: newest.command.clone(),
        });
        self.want = Want::Idle;
    }

    /// The section [`Queue::next`] named could not be spawned: held for the
    /// next trigger.
    pub fn spawn_failed(&mut self) {
        self.want = Want::Held;
    }

    /// When the running one is to be abandoned, if one runs.
    pub fn deadline(&self) -> Option<Instant> {
        self.running.as_ref().map(|run| run.started + self.patience)
    }

    /// Abandons the running one if its deadline has passed at `now`,
    /// returning its pid (for the log). It stays tracked, to be reaped.
    pub fn expire(&mut self, now: Instant) -> Option<u32> {
        let run = self.running.as_ref()?;
        if now < run.started + self.patience {
            return None;
        }
        let run = self.running.take()?;
        let pid = run.pid;
        self.abandoned.push(run);
        Some(pid)
    }

    /// Whether any pid is tracked (the reaper's fast path).
    pub fn is_empty(&self) -> bool {
        self.running.is_none() && self.abandoned.is_empty()
    }

    /// The pid in flight, if any.
    pub fn running_pid(&self) -> Option<u32> {
        self.running.as_ref().map(|run| run.pid)
    }

    /// Asks `wait` about every tracked pid, and applies the rules above to
    /// each that ended, calling `ended` for each (in-flight run first). No
    /// allocation: the abandoned list is filtered in place.
    pub fn reap(&mut self, mut wait: impl FnMut(u32) -> Waited, mut ended: impl FnMut(Ended)) {
        if let Some(run) = &self.running
            && let Some(exit) = exit_of(wait(run.pid))
            && let Some(run) = self.running.take()
        {
            self.ran(&run, exit, false);
            ended(Ended {
                pid: run.pid,
                command: run.command,
                exit,
                abandoned: false,
            });
        }
        let mut index = 0;
        while index < self.abandoned.len() {
            let pid = self.abandoned[index].pid;
            match exit_of(wait(pid)) {
                None => index += 1,
                Some(exit) => {
                    let run = self.abandoned.swap_remove(index);
                    self.ran(&run, exit, true);
                    ended(Ended {
                        pid,
                        command: run.command,
                        exit,
                        abandoned: true,
                    });
                }
            }
        }
    }

    /// Applies one run's end to what is wanted.
    fn ran(&mut self, run: &Run, exit: Exit, abandoned: bool) {
        let Some(newest) = &self.newest else {
            return;
        };
        let newest_seq = newest.seq;
        if abandoned {
            // A trigger: a held section is retried now.
            self.trigger();
            if run.seq != newest_seq {
                // An older section may have landed after the newest: run
                // the newest again.
                self.want = Want::Now;
            } else if exit.succeeded() {
                // The newest is applied, by this run. A retry still in
                // flight is left to finish; nothing more is needed.
                if self.want == Want::Now && self.running.is_none() {
                    self.want = Want::Idle;
                }
            } else if self.want == Want::Idle && self.running.is_none() {
                self.want = Want::Held;
            }
            return;
        }
        if run.seq == newest_seq && !exit.succeeded() && self.want == Want::Idle {
            self.want = Want::Held;
        }
    }

    #[cfg(test)]
    pub fn set_patience(&mut self, patience: Duration) {
        self.patience = patience;
    }

    #[cfg(test)]
    pub fn want(&self) -> Want {
        self.want
    }

    #[cfg(test)]
    pub fn abandoned_count(&self) -> usize {
        self.abandoned.len()
    }
}

fn exit_of(waited: Waited) -> Option<Exit> {
    match waited {
        Waited::Running => None,
        Waited::Exited(status) => Some(Exit::from_status(status)),
        Waited::Gone => Some(Exit::Unknown),
    }
}
