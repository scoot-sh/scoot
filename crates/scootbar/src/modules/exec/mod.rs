//! The `exec` module: run a command and show what it prints.
//!
//! ```toml
//! right = ["weather", "clock"]
//! [exec.weather]
//! command = ["sh", "-c", "while :; do curl -s 'wttr.in?format=1'; sleep 600; done"]
//! placeholder = "..."          # shown until the first line; optional
//! format = "text"              # or "json": one object per line, see `payload`
//! ```
//!
//! **Streaming, not polling.** The command runs once and the bar waits on
//! its stdout (a nonblocking pipe the loop polls): a script that can wait
//! for an event prints when it has one, and the bar does nothing in
//! between, which is the lightest way to run one and the one that cannot
//! leak or spin the way an interval-polled script does. There is no
//! `interval` key on purpose: a script that has to poll writes
//! `while :; do ...; sleep N; done` (print first, so the module shows its
//! first line at once and not after the first sleep), which moves the choice,
//! and its cost, into a script the user can see. A command that prints once
//! and exits (`["date"]`) is a poll too, by the restart rule below, backing
//! off to once a minute. Each line is one update ([`super::payload`]); the
//! last line of a read is what is shown.
//!
//! ## What keeps it bounded
//!
//! - **Lines**: at most [`lines::MAX_LINE`] bytes; a longer one is dropped
//!   whole, with one warning a second at most, never truncated and never
//!   held ([`lines`]). A child cannot make the bar buffer more than that.
//! - **Rate**: at most one read of [`CHUNK`] bytes every [`FRAME`] (16 ms)
//!   once data keeps coming. While it is held the bar does not even poll
//!   the pipe, which fills and blocks the child (kernel back-pressure): a
//!   child printing as fast as it can costs the bar about 60 small reads a
//!   second, no memory, and the update shown is the last one read. A child
//!   that prints once a minute costs nothing between lines.
//! - **Restarts**: a command that exits is started again after 1 s, then 2,
//!   4, ... up to 60 s; a run of 30 s or more starts the sequence over
//!   ([`backoff`]). One that cannot start (no such program) takes the same
//!   path. Said on stderr, at most one warning a second: a restart is
//!   normally one line, but one that follows another warning inside the
//!   second is not said.
//! - **Children**: reaped the moment they exit (the pidfd wakes the loop,
//!   `try_wait` collects), killed with their whole process group when the
//!   module is dropped (a reload, a refused start) and when the command
//!   exits (so a worker it backgrounded does not pile up over restarts).
//!   The command's stdin is `/dev/null`, its stderr is the bar's own
//!   (a script's complaints reach the journal), and it inherits none of
//!   the descriptors the bar opens (one a launcher left open when it
//!   started the bar reaches it, as it does any child). **It ends with the bar, however the bar ends**:
//!   a clean exit kills the group by `Drop`, and a killed or crashed bar is
//!   covered by the kernel's parent-death signal, armed by [`guard`].
//! - **Count**: at most [`super::custom::MAX_EXEC`] are placed; each costs
//!   at most two polled fds and one `timerfd`.
//!
//! Nothing is allocated for a text line once the module is warm; a JSON
//! line allocates its parse tree.

mod backoff;
pub mod guard;
mod lines;
mod timer;

#[cfg(test)]
mod tests;

use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdout, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use rustix::event::PollFlags;
use rustix::io::Errno;
use rustix::process::{Pid, PidfdFlags, Signal, kill_process_group, pidfd_open};

use super::payload::{Format, Shown, parse_line};
use super::{Module, OutputView, Sources, Update, View};
use crate::print::warn;
use backoff::Backoff;
pub use backoff::Restart;
use lines::Lines;
use timer::Timer;

/// The shortest time between two reads of a child's output while it keeps
/// printing: one 60 Hz frame.
pub const FRAME: Duration = Duration::from_millis(16);

/// How much one read takes.
const CHUNK: usize = 4096;

/// How much of what a child printed before it exited is read after it,
/// at most: a grandchild still writing to the pipe cannot keep the bar
/// reading.
const MAX_DRAIN: usize = 64 * 1024;

/// The longest between two warnings from one module.
const SAY_EVERY: Duration = Duration::from_secs(1);

/// An `exec` module's options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The program and its arguments: never run through a shell.
    pub command: Vec<String>,
    pub format: Format,
    /// Shown until the first line (sanitized and bounded as any text).
    pub placeholder: String,
    pub restart: Restart,
}

/// A started child and the pipe it is read from.
struct Running {
    child: Child,
    out: ChildStdout,
    /// Readable when the child exits; `None` on a kernel without pidfds
    /// (the pipe's end of file then says so).
    pidfd: Option<OwnedFd>,
    started: Instant,
    lines: Lines,
    /// The pipe's end of file was read: nothing more will come.
    eof: bool,
    /// The earliest the next read may be, and whether the pipe is held
    /// back until then (the timer, not the pipe, is polled).
    next_read: Instant,
    held: bool,
}

impl Drop for Running {
    fn drop(&mut self) {
        kill_group(&self.child);
        // A killed process is reaped at once; the wait cannot block.
        let _ = self.child.wait();
    }
}

enum Run {
    /// No child: one is started when `at` comes.
    Waiting {
        at: Instant,
    },
    Running(Box<Running>),
}

pub struct Exec {
    id: &'static str,
    settings: Settings,
    shown: Shown,
    /// Where a parsed line goes before it is compared with `shown`.
    scratch: Shown,
    timer: Timer,
    backoff: Backoff,
    run: Run,
    last_said: Option<Instant>,
}

/// Starts the module `id` (nothing runs until the loop first wakes it).
pub fn start(id: &'static str, settings: &Settings) -> Result<Box<dyn Module>, String> {
    let timer = Timer::new().map_err(|e| format!("cannot create its timer: {e}"))?;
    let now = Instant::now();
    // The first start is the restart path with no wait: the loop wakes the
    // module right after the first frame, so a slow `fork` never delays it.
    timer
        .arm(Duration::ZERO)
        .map_err(|e| format!("cannot arm its timer: {e}"))?;
    Ok(Box::new(Exec {
        id,
        shown: Shown::text(&settings.placeholder),
        scratch: Shown::default(),
        timer,
        backoff: Backoff::new(settings.restart),
        settings: settings.clone(),
        run: Run::Waiting { at: now },
        last_said: None,
    }))
}

/// Kills `child`'s whole process group (it leads one). After the leader was
/// reaped the group number is still the group's while any member lives (a
/// worker it left); with none left a pid is reused only once the pid space
/// wraps, which cannot happen between the reap and this call.
fn kill_group(child: &Child) {
    let _ = kill_process_group(Pid::from_child(child), Signal::KILL);
}

impl Exec {
    /// Whether a warning may be said now: at most one a second.
    fn may_say(&mut self, now: Instant) -> bool {
        if self
            .last_said
            .is_some_and(|last| now.saturating_duration_since(last) < SAY_EVERY)
        {
            return false;
        }
        self.last_said = Some(now);
        true
    }

    /// Starts the command; on failure, schedules the next try.
    fn spawn(&mut self, now: Instant) {
        match self.try_spawn(now) {
            Ok(running) => self.run = Run::Running(Box::new(running)),
            Err(why) => {
                let wait = self.backoff.after(Duration::ZERO);
                self.wait_for(now, wait);
                warn(format_args!(
                    "scootbar: exec module `{}`: {why}; trying again in {}",
                    self.id,
                    human(wait)
                ));
            }
        }
    }

    fn try_spawn(&self, now: Instant) -> Result<Running, String> {
        let (program, args) = self.settings.command.split_first().ok_or("no command")?;
        let mut command = guard::command(program, args);
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .process_group(0)
            .spawn()
            .map_err(|e| guard::spawn_error(&command, program, &e))?;
        let Some(out) = child.stdout.take() else {
            kill_group(&child);
            let _ = child.wait();
            return Err("no pipe to read".to_owned());
        };
        if let Err(e) = rustix::fs::fcntl_setfl(&out, rustix::fs::OFlags::NONBLOCK) {
            kill_group(&child);
            let _ = child.wait();
            return Err(format!("cannot make its pipe nonblocking: {e}"));
        }
        let pidfd = pidfd_open(Pid::from_child(&child), PidfdFlags::empty()).ok();
        Ok(Running {
            child,
            out,
            pidfd,
            started: now,
            lines: Lines::default(),
            eof: false,
            next_read: now,
            held: false,
        })
    }

    /// No child for `wait`: the timer wakes the loop when it is over.
    fn wait_for(&mut self, now: Instant, wait: Duration) {
        self.run = Run::Waiting { at: now + wait };
        if self.timer.arm(wait).is_err() {
            // Without a timer the module would never start again: the
            // wait is spent on the next wake of anything, which is the
            // best that is left.
            warn(format_args!(
                "scootbar: exec module `{}`: cannot arm its timer",
                self.id
            ));
        }
    }

    /// Applies the last line `chunk` completes, if any; reports a change.
    fn apply(&mut self, chunk: &[u8], now: Instant) -> Update {
        let Run::Running(running) = &mut self.run else {
            return Update::Unchanged;
        };
        // Only the last valid line of a read is shown: an update is a state,
        // not an event, so earlier ones in the same read are already stale.
        let format = self.settings.format;
        let mut invalid: Option<String> = None;
        let mut shown_any = false;
        running.lines.feed(chunk, |line| {
            match parse_line(format, line, &mut self.scratch) {
                Ok(()) => shown_any = true,
                Err(why) => invalid = Some(why.to_string()),
            }
        });
        let dropped = running.lines.take_dropped();
        if dropped > 0 && self.may_say(now) {
            warn(format_args!(
                "scootbar: exec module `{}`: dropped {dropped} line(s) longer than {} bytes",
                self.id,
                lines::MAX_LINE
            ));
        }
        if let Some(why) = invalid {
            if self.may_say(now) {
                warn(format_args!(
                    "scootbar: exec module `{}`: ignored a line: {why}",
                    self.id
                ));
            }
        }
        if shown_any && self.scratch != self.shown {
            std::mem::swap(&mut self.scratch, &mut self.shown);
            return Update::Changed;
        }
        Update::Unchanged
    }

    /// Reads what the pipe has, at most one chunk and not before the
    /// frame is over.
    fn read(&mut self, now: Instant) -> Update {
        let Run::Running(running) = &mut self.run else {
            return Update::Unchanged;
        };
        running.held = false;
        if running.eof {
            return Update::Unchanged;
        }
        if now < running.next_read {
            // Data again inside the frame: hold the pipe until it is over.
            running.held = true;
            let wait = running.next_read - now;
            if self.timer.arm(wait).is_err() {
                running.held = false;
            }
            return Update::Unchanged;
        }
        let mut chunk = [0u8; CHUNK];
        match rustix::io::read(&running.out, &mut chunk) {
            Ok(0) => {
                running.eof = true;
                Update::Unchanged
            }
            Ok(n) => {
                running.next_read = now + FRAME;
                let Some(data) = chunk.get(..n) else {
                    return Update::Unchanged;
                };
                self.apply(data, now)
            }
            Err(Errno::AGAIN | Errno::INTR) => Update::Unchanged,
            Err(_) => {
                // The pipe broke: nothing more can come; the exit check
                // that follows decides what to do about the child.
                running.eof = true;
                Update::Unchanged
            }
        }
    }

    /// Whether the child exited (reaping it), and if so what follows.
    fn check_exit(&mut self, now: Instant) -> Update {
        let Run::Running(running) = &mut self.run else {
            return Update::Unchanged;
        };
        let status: Option<ExitStatus> = match running.child.try_wait() {
            Ok(status) => status,
            // A child that cannot be waited on is gone for our purposes.
            Err(_) => Some(ExitStatus::default()),
        };
        let exited = match status {
            Some(status) => Some(status),
            // A kernel with no pidfds learns of the exit by the pipe's end
            // of file, which a child can also reach by closing it alive:
            // kill it, so the bar's view and the process agree.
            None if running.eof && running.pidfd.is_none() => {
                kill_group(&running.child);
                running.child.wait().ok()
            }
            None => None,
        };
        let Some(status) = exited else {
            return Update::Unchanged;
        };
        // Whatever it printed last, and a line it did not end.
        let mut changed = Update::Unchanged;
        let mut drained = 0;
        while drained < MAX_DRAIN {
            let Run::Running(running) = &mut self.run else {
                break;
            };
            let mut chunk = [0u8; CHUNK];
            match rustix::io::read(&running.out, &mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    drained += n;
                    if let Some(data) = chunk.get(..n) {
                        if self.apply(data, now) == Update::Changed {
                            changed = Update::Changed;
                        }
                    }
                }
            }
        }
        let Run::Running(running) = &mut self.run else {
            return changed;
        };
        let (lasted, format) = (
            now.saturating_duration_since(running.started),
            self.settings.format,
        );
        let mut last = None;
        running.lines.finish(|line| last = Some(line.to_vec()));
        // The leader is gone: so is anything it left behind (dropped with
        // `running` below, by the group kill in its `Drop`).
        let wait = self.backoff.after(lasted);
        if let Some(line) = last {
            if parse_line(format, &line, &mut self.scratch).is_ok() && self.scratch != self.shown {
                std::mem::swap(&mut self.scratch, &mut self.shown);
                changed = Update::Changed;
            }
        }
        if self.may_say(now) {
            let why = status.code().and_then(guard::meaning).unwrap_or("");
            warn(format_args!(
                "scootbar: exec module `{}`: the command ended ({status}{why}); \
                 starting it again in {}",
                self.id,
                human(wait)
            ));
        }
        self.wait_for(now, wait);
        changed
    }
}

/// A wait for a message: `1s`, `250ms`.
fn human(wait: Duration) -> String {
    if wait.as_secs() >= 1 {
        format!("{}s", wait.as_secs())
    } else {
        format!("{}ms", wait.as_millis())
    }
}

impl Module for Exec {
    fn sources<'fd>(&'fd self, sources: &mut Sources<'_, 'fd>) {
        match &self.run {
            Run::Waiting { .. } => {
                sources.add(self.timer.as_fd(), PollFlags::IN);
            }
            Run::Running(running) => {
                if running.held {
                    sources.add(self.timer.as_fd(), PollFlags::IN);
                } else if !running.eof {
                    sources.add(running.out.as_fd(), PollFlags::IN);
                }
                if let Some(pidfd) = &running.pidfd {
                    sources.add(pidfd.as_fd(), PollFlags::IN);
                }
            }
        }
    }

    /// Any of its fds: which one is read off the state, not the number
    /// (the set it polls changes as it runs), so a wake handles all of
    /// them, each a cheap nonblocking check.
    fn on_ready(&mut self, _source: usize, _events: PollFlags) -> Update {
        let _ = self.timer.fired();
        let now = Instant::now();
        if let Run::Waiting { at } = self.run {
            if now >= at {
                self.spawn(now);
            } else if self.timer.arm(at - now).is_err() {
                self.run = Run::Waiting { at: now };
            }
        }
        let mut changed = self.read(now);
        if self.check_exit(now) == Update::Changed {
            changed = Update::Changed;
        }
        changed
    }

    fn view(&self, _: &OutputView<'_>, view: &mut View) {
        self.shown.write(view);
    }
}
