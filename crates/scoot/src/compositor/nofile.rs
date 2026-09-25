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
//! on a table of 8192 or more. So [`raise`] lifts the soft limit toward the
//! hard limit once, at startup, the way other compositors (niri among them)
//! do.
//!
//! **Children get the original back.** A program that still uses `select()`
//! cannot watch an fd numbered 1024 or above (`FD_SETSIZE`), and one that
//! inherits a raised soft limit may open such fds and break in ways that
//! have nothing to do with scoot. Every process scoot starts runs with the
//! soft limit scoot itself was started with: [`restore_for_child`] on the
//! `Command` in `State::spawn` (the one path keybindings, IPC `spawn`,
//! `[autostart]` and the session command all go through), and
//! `with_original_soft` around the XWayland launch, whose `Command` is
//! built inside Smithay with no `pre_exec` hook.
//!
//! **The cap, [`RAISED_SOFT_CAP`] = 65536.** Raising costs nothing until
//! fds are used (the kernel grows a process's fd table on demand), and
//! epoll's cost is per registered fd, not per limit. What a raised limit
//! does change is how much one local client can make this process hold
//! before fd pressure (`fd_pressure.rs`) turns newcomers away: the pressure
//! line is the table minus a 128-fd reserve, so on a 524288-fd hard limit
//! (the dev VM's) an uncapped raise would let clients park half a million
//! fds, each pinning a kernel `struct file`, and make every pressure
//! observation (a readdir of `/proc/self/fd`, O(open fds)) that much
//! slower. 65536 is 64 times the default: room for 64 connections each at
//! every per-client bound at once, including the 1024-fd queue cap, while
//! keeping an observation of a full table in the low milliseconds.
//!
//! **A hard limit too low to raise** (a container started with
//! `--ulimit nofile=1024:1024`, say) leaves the soft limit where it is; the
//! startup log says so, and every margin in `fd_pressure.rs` is the one
//! documented there for a 1024-fd table.

use std::os::unix::process::CommandExt;
use std::process::Command;
use std::sync::OnceLock;

/// The most [`raise`] lifts the soft limit to, whatever the hard limit. See
/// the module doc for the number.
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
    fn raised(&self) -> bool {
        self.soft != self.original_soft
    }
}

static RAISED: OnceLock<Option<Limits>> = OnceLock::new();

/// Raises the soft `RLIMIT_NOFILE` to [`target_soft`], once per process, and
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
    let soft_now = if target > soft {
        match set(target, hard) {
            Ok(()) => target,
            Err(error) => {
                tracing::warn!(
                    soft,
                    hard,
                    target,
                    %error,
                    "cannot raise the RLIMIT_NOFILE soft limit; staying at {soft}"
                );
                soft
            }
        }
    } else {
        soft
    };
    let queue_cap = crate::compositor::fd_pressure::backend_queued_fds(soft_now);
    if soft_now > soft {
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

/// The soft limit [`raise`] aims for: the hard limit, capped at
/// [`RAISED_SOFT_CAP`], and never below the current soft limit (a process
/// started with more keeps it). Total on any input, `RLIM_INFINITY`
/// included.
pub(crate) fn target_soft(soft: u64, hard: u64) -> u64 {
    hard.min(RAISED_SOFT_CAP).max(soft)
}

/// Makes `command`'s child start with the soft limit this process was
/// started with, when [`raise`] changed it; otherwise leaves `command`
/// alone. The hard limit is untouched.
pub(crate) fn restore_for_child(command: &mut Command) {
    let Some(limits) = raised().filter(Limits::raised) else {
        return;
    };
    let original = rlimit(limits.original_soft, limits.hard);
    // SAFETY: the closure runs in the child between `fork` and `exec`, where
    // only async-signal-safe calls are allowed. It makes one: `setrlimit`,
    // on a value built before the fork and moved in by copy. It allocates
    // nothing and takes no lock.
    unsafe {
        command.pre_exec(move || {
            if libc::setrlimit(libc::RLIMIT_NOFILE, &original) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
}

/// Runs `spawn` with this process's soft limit set back to the original for
/// its duration, for a child whose `Command` scoot cannot reach (the
/// XWayland server, built inside Smithay's `XWayland::spawn`).
///
/// The limit is process-wide, so this is only for startup, where nothing
/// else is opening fds: it is skipped (the child inherits the raised limit,
/// with a warning) when this process already has as many fds open as the
/// original limit allows, since every fd `spawn` itself opens would then
/// fail.
#[cfg(feature = "xwayland")]
pub(crate) fn with_original_soft<T>(spawn: impl FnOnce() -> T) -> T {
    let Some(limits) = raised().filter(Limits::raised) else {
        return spawn();
    };
    match open_fds() {
        Some(open) if open < limits.original_soft => {}
        open => {
            tracing::warn!(
                ?open,
                original = limits.original_soft,
                "too many fds open to lower the fd limit for this child; it inherits {}",
                limits.soft
            );
            return spawn();
        }
    }
    if let Err(error) = set(limits.original_soft, limits.hard) {
        tracing::warn!(%error, "cannot lower the fd limit for this child; it inherits {}", limits.soft);
        return spawn();
    }
    let out = spawn();
    if let Err(error) = set(limits.soft, limits.hard) {
        tracing::warn!(
            %error,
            "cannot restore the raised fd limit after starting a child; staying at {}",
            limits.original_soft
        );
    }
    out
}

/// This process's open fds, or `None` if `/proc/self/fd` cannot be read.
#[cfg(feature = "xwayland")]
fn open_fds() -> Option<u64> {
    Some(std::fs::read_dir("/proc/self/fd").ok()?.count() as u64)
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
