//! The process's fd limit: raised at startup, restored for every child.
//!
//! A compositor holds an fd for every client socket, every shm pool and
//! dma-buf plane a client hands over, every sync timeline, and, below all
//! of that, every fd a client has sent that no request has claimed yet
//! (wayland-backend's received-fd queue; see `fd_pressure.rs`). The default
//! soft `RLIMIT_NOFILE` of 1024 makes that table the tightest resource in
//! the process: every fd-exhaustion bound scoot carries was sized against
//! it, and the wayland-backend fork's cap on unclaimed fds (one eighth of
//! the soft limit, 128..=1024) can only reach libwayland-server's own 1024
//! on a table of 8192 or more. So [`raise`] sets the soft limit to the hard
//! limit (capped, below) once, at startup, the way other compositors (niri
//! among them) raise theirs.
//!
//! **Children get the original back.** A program that still uses `select()`
//! cannot watch an fd numbered 1024 or above (`FD_SETSIZE`), and one that
//! inherits a raised soft limit may open such fds and break in ways that
//! have nothing to do with scoot. Every process scoot starts through
//! `State::spawn` (the one path keybindings, IPC `spawn`, `[autostart]` and
//! the session command all go through) runs with the soft limit scoot itself
//! was started with: [`restore_for_child`] sets it in the child before
//! `exec`.
//!
//! **The XWayland server is the exception, deliberately.** Its `Command` is
//! built inside Smithay's `XWayland::spawn`, with no `pre_exec` hook, and
//! restoring the limit around that call would not reach it anyway: Xwayland
//! raises its own soft limit to the hard limit at startup
//! (`try_raising_nofile_limit` in upstream `hw/xwayland/xwayland.c`, skipped
//! only for an explicit `-lf`), because an X server holds an fd per X client
//! and polls with epoll. Measured on the dev VM: Xwayland 24.1.13 started by
//! scoot runs at soft 524288, its hard limit, whatever it inherited. Putting
//! 1024 back first would have meant lowering this whole process's limit for
//! the length of the spawn, for nothing.
//!
//! **The cap, [`RAISED_SOFT_CAP`] = 65536**, applied in both directions:
//! a soft limit above it is lowered to it too (Docker before 25 starts
//! containers at 1048576:1048576), and children get their original back
//! either way. Raising costs nothing until fds are used (the kernel grows a
//! process's fd table on demand), and epoll's cost is per registered fd, not
//! per limit. What the size does change is how much local clients can make
//! this process hold before fd pressure (`fd_pressure.rs`) turns newcomers
//! away (the table minus a 128-fd reserve), each fd pinning a kernel
//! `struct file`, and what observing the table costs: a readdir of
//! `/proc/self/fd`, linear in open fds, ~7 ms at 60000. That observation is
//! cached (`fd_pressure::table`: reused for 20 times its cost, so at most
//! ~5% of loop time), so a full 65536-entry table costs one ~7 ms readdir
//! per ~140 ms, not one per event. Measured with that cache on the dev VM
//! (`scripts/fd-storm/run.sh`): 58 connections parking 1008 fds each (58540
//! open) plus a 4000-connection burst gave an ordinary client a worst
//! round-trip wait of 101-110 ms, the same as the burst with nothing parked
//! (109 ms) and below `main`'s 361 ms on a 1024 table; uncached it was
//! 36 s. 65536 is 64 times the default: room for 64 connections each at
//! every per-client bound at once, including the 1024-fd queue cap.
//!
//! **A hard limit too low to raise** (a container started with
//! `--ulimit nofile=1024:1024`, say) leaves the soft limit where it is; the
//! startup log says so, and every margin in `fd_pressure.rs` is the one
//! documented there for a 1024-fd table.

use std::os::unix::process::CommandExt;
use std::process::Command;
use std::sync::OnceLock;

/// The soft limit [`raise`] sets wherever the hard limit allows it, whether
/// that raises or lowers the one scoot started with. See the module doc for
/// the number.
pub(crate) const RAISED_SOFT_CAP: u64 = 65536;

/// What [`raise`] found and did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// The soft limit the process was started with: what children get.
    pub original_soft: u64,
    /// The soft limit after the raise (equal to `original_soft` when there
    /// was nothing to raise, or the raise failed).
    pub soft: u64,
    /// The hard limit, unchanged.
    pub hard: u64,
}

impl Limits {
    /// Whether [`raise`] moved the soft limit (up, or down to the cap).
    fn changed(&self) -> bool {
        self.soft != self.original_soft
    }
}

static RAISED: OnceLock<Option<Limits>> = OnceLock::new();

/// Sets the soft `RLIMIT_NOFILE` to [`target_soft`], once per process, and
/// logs what it did; later calls return the first call's answer. `None`
/// when the limit could not be read at all (nothing was changed, and
/// children inherit whatever the process has).
///
/// Called at the top of `compositor::run`, before anything opens more than a
/// handful of fds, and again (a no-op there) from `State::new`, so that a
/// test harness's `State` runs with the limit a session runs with.
pub(crate) fn raise() -> Option<Limits> {
    *RAISED.get_or_init(raise_once)
}

/// The answer of an earlier [`raise`], without raising.
fn raised() -> Option<Limits> {
    RAISED.get().copied().flatten()
}

fn raise_once() -> Option<Limits> {
    let Some((soft, hard)) = get() else {
        tracing::warn!("cannot read RLIMIT_NOFILE; leaving the fd limit as it is");
        return None;
    };
    let target = target_soft(soft, hard);
    let soft_now = if target != soft {
        match set(target, hard) {
            Ok(()) => target,
            Err(error) => {
                tracing::warn!(
                    soft,
                    hard,
                    target,
                    %error,
                    "cannot set the RLIMIT_NOFILE soft limit; staying at {soft}"
                );
                soft
            }
        }
    } else {
        soft
    };
    let queue_cap = crate::compositor::fd_pressure::backend_queued_fds(soft_now);
    if soft_now < soft {
        tracing::info!(
            from = soft,
            to = soft_now,
            hard,
            unclaimed_fd_cap = queue_cap,
            "lowered the fd limit (RLIMIT_NOFILE soft) to scoot's cap; children get {soft} back"
        );
    } else if soft_now > soft {
        tracing::info!(
            from = soft,
            to = soft_now,
            hard,
            unclaimed_fd_cap = queue_cap,
            "raised the fd limit (RLIMIT_NOFILE soft); children get {soft} back"
        );
    } else {
        tracing::info!(
            soft,
            hard,
            unclaimed_fd_cap = queue_cap,
            "fd limit not raised (the hard limit allows no more); every fd margin is the \
             one for a {soft}-fd table, and a client may leave {queue_cap} received fds \
             unclaimed"
        );
    }
    Some(Limits {
        original_soft: soft,
        soft: soft_now,
        hard,
    })
}

/// The soft limit [`raise`] sets: the hard limit, capped at
/// [`RAISED_SOFT_CAP`], also when that means lowering a soft limit the
/// process was started with (Docker before 25 starts containers at
/// 1048576:1048576, where a raise that only ever went up would leave scoot a
/// million-entry table). Children get the original back either way. Total on
/// any input, `RLIM_INFINITY` included; `_soft` is taken so a caller cannot
/// mistake this for a function of the hard limit alone by accident.
pub(crate) fn target_soft(_soft: u64, hard: u64) -> u64 {
    hard.min(RAISED_SOFT_CAP)
}

/// Makes `command`'s child start with the soft limit this process was
/// started with, when [`raise`] changed it; otherwise leaves `command`
/// alone. The hard limit is untouched.
///
/// Never fails the spawn: the child's `setrlimit` result is ignored. It can
/// only fail if something outside scoot lowered its hard limit below the
/// original soft limit since startup, which is checked here first (one
/// `getrlimit` per spawn): the child then gets the original clamped to the
/// current hard limit, with a warning, rather than no child at all.
pub(crate) fn restore_for_child(command: &mut Command) {
    let Some(limits) = raised().filter(Limits::changed) else {
        return;
    };
    let hard = get().map_or(limits.hard, |(_, hard)| hard);
    let soft = if hard < limits.original_soft {
        tracing::warn!(
            original = limits.original_soft,
            hard,
            "the hard fd limit is now below the one scoot started with; this child gets {hard}"
        );
        hard
    } else {
        limits.original_soft
    };
    let original = rlimit(soft, hard);
    // SAFETY: the closure runs in the child between `fork` and `exec`, where
    // only async-signal-safe calls are allowed. It makes one: `setrlimit`,
    // on a value built before the fork and moved in by copy. It allocates
    // nothing and takes no lock, and its result is ignored (see above).
    unsafe {
        command.pre_exec(move || {
            libc::setrlimit(libc::RLIMIT_NOFILE, &original);
            Ok(())
        });
    }
}

fn rlimit(soft: u64, hard: u64) -> libc::rlimit {
    libc::rlimit {
        rlim_cur: soft as libc::rlim_t,
        rlim_max: hard as libc::rlim_t,
    }
}

/// `(soft, hard)`, `RLIM_INFINITY` as `u64::MAX`.
fn get() -> Option<(u64, u64)> {
    // SAFETY: `getrlimit` writes exactly one `struct rlimit` through a live
    // pointer to one, and returns nonzero on failure without touching it
    // (the zeroed value is then never read).
    let mut limits: libc::rlimit = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limits) } != 0 {
        return None;
    }
    Some((limits.rlim_cur as u64, limits.rlim_max as u64))
}

fn set(soft: u64, hard: u64) -> std::io::Result<()> {
    let limits = rlimit(soft, hard);
    // SAFETY: a plain value read through a live pointer; on failure the
    // limit is unchanged.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limits) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests;
