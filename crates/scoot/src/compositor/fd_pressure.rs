//! Compositor-wide file-descriptor pressure: the global ceiling.
//!
//! Every other bound in this compositor is per connection (512 live
//! `wl_buffer`s, 128 live pools, 8 binds, 16 capture frames, 64 IPC slots,
//! and where explicit sync is offered 128 live syncobj timelines and 64
//! outstanding acquire waits -- see `drm_syncobj.rs`),
//! while the fd table they all draw from is process-global (`RLIMIT_NOFILE`
//! 1024 on the dev VM). Two connections inside every per-connection bound
//! hold ~2 x 641 fds against it with nothing tripped -- the residual
//! `docs/backlog/resolved/wayland-connection-cap-done.md` fixed the kill
//! half of and left here. This module is the shared ceiling:
//! one observation of the table ([`table`]) and one predicate
//! ([`Table::pressured`]) that three enforcement sites read.
//!
//! ## The three responses
//!
//! - **Wayland accept** (`wayland_accept::admit`): a newcomer past the
//!   ceiling is dropped -- an immediate EOF, the same wire shape as the
//!   `EMFILE` shed, there being no protocol channel for a reason.
//! - **IPC accept** (`ipc::accept`): a newcomer past the ceiling is refused
//!   with a reason naming the pressure, the same shape as the 64-slot cap
//!   refusal -- IPC, unlike Wayland, has a channel for it.
//! - **Creation guards** (`dispatch.rs`'s pool/buffer claims): a creation
//!   past a per-client *grace* ([`PRESSURE_GRACE_BUFFERS`] /
//!   [`PRESSURE_GRACE_POOLS`]) while the table is pressured is refused with
//!   the same protocol error the per-connection cap would post. The grace is
//!   what makes this a ceiling rather than a lottery: a bar holding 2
//!   buffers is never refused for another client's greed, only a client
//!   already holding past-grace is ever killed, and a killed client's
//!   disconnect frees what it held, so the pressure it caused lifts with it.
//!
//! What is deliberately *not* here: any creation-time wait, queue or silent
//! ignore (no protocol channel carries "retry later" on these interfaces,
//! and a silent ignore leaves the uninitialized object that panics the
//! compositor -- the argument `dispatch.rs` already makes), and any
//! connection-count cap (any usable count admits the killing pair; see the
//! verdict above).
//!
//! ## The numbers, all measured
//!
//! - Idle `--headless`: **14 fds** (`/proc/PID/fd`, dev VM, 2026-09-18).
//! - Plus one `foot` window with a shell: **17** (delta +3: one socket, two
//!   pools; buffers retain pool fds rather than opening new ones). The
//!   shell's own 16 fds live in its process, not this table.
//! - A login storm (bar, panels, launcher, a handful of apps -- ~30
//!   connections at ~3-4 fds each) lands near **100-150**, by the same
//!   per-connection arithmetic, not by a live 30-client session.
//! - One connection at every hard cap: 512 buffers + 128 pools + 1 socket
//!   = **~641 fds**. One such connection plus a normal session (~750) never
//!   trips anything here, correctly: a single connection cannot exhaust the
//!   table on its own.
//! - Where explicit sync is offered (the `--tty` GPU scanout tier), each
//!   live syncobj timeline holds the client's fd and each outstanding
//!   acquire wait an eventfd, so one connection at every cap there is
//!   641 + 128 + 64 = **~833 fds** -- still short of a 1024-fd table on its
//!   own, but one such connection plus a busy session does reach the
//!   reserve. The kill then lands on it, the contributor: both of those
//!   counts have their own pressure graces (32 timelines, 16 waits),
//!   enforced the same way as the buffer and pool graces.
//!
//! [`RESERVE_FDS`] is 128: shed/refuse once fewer than 128 fds stand free
//! (used past 896 of 1024). That is ~6x above the reasoned login storm and
//! still leaves room for a whole greedy connection's transient burst, while
//! the two-greedy fill trips the creation guard with the second greedy near
//! ~370 of its 512 buffers -- before exhaustion, not after it.
//!
//! [`MIN_TABLE_FDS`] is 512: below it the guard stays off entirely
//! ([`table`] returns `None`, every site fails open) and the `EMFILE` shed
//! is the only backstop. A table that small cannot tell pressure from a
//! busy session -- enforcing a 128 reserve on a 256-fd table would shed a
//! normal login storm -- so the honest answer is no guard rather than a
//! hair-trigger one.
//!
//! The graces (128 buffers = 64x the measured single-window floor of 2 and
//! ~2x the heaviest reasoned legitimate use of ~60 for a 20-window browser
//! at triple buffering; 64 pools = 32x the floor and ~1.6x the reasoned ~40)
//! bite only *during* genuine pressure, which a legitimate session never
//! produces (see above): holding past-grace while the table is 7/8 full
//! means contributing to the pressure, which is what justifies the kill.
//! Two connections sitting exactly at grace hold 2 x (128 + 64 + 1) + 14
//! baseline = 400 fds -- pressure still requires someone past grace, so the
//! refusal always lands on a contributor. Stated exactly: `live > grace`
//! admits the grace+1-th unit, so two connections at the permitted maximum
//! hold 2 x (129 + 65 + 1) + 14 = 404 fds, 4 above the "two at grace"
//! figure -- negligible, but the pins in `dispatch.rs` hold it there.
//!
//! ## Observation cost and disciplines
//!
//! [`table`] costs one `getrlimit` plus one `/proc/self/fd` readdir --
//! ~8us per call on the dev VM (debug build, 2000-call sample; release is
//! faster), no steady-state cost anywhere: the accept sites run it once per
//! *connection* (not per frame or request), and the creation sites only
//! once a client is already past its grace (a `HashMap` lookup short-
//! circuits everything under it).
//!
//! `table()` allocates (`read_dir`), so the fork-child discipline from
//! `ipc::accept` applies: never call it from `drain`, `shed_one`,
//! `classify`, or anything a forked exhaustion-test child executes. Every
//! current call site (the two accept callbacks, the dispatch creation
//! guards) runs on the loop thread, never in a forked child.
//!
//! Unknown means calm: any observation failure (`getrlimit` error, an
//! infinite limit, an unreadable `/proc`) returns `None`, and every site
//! admits on `None`. Shedding on unknown would deny innocents for a
//! broken gauge; the `EMFILE` shed still catches real exhaustion underneath.

/// How many free fds must remain before newcomers shed and past-grace
/// creations refuse. See the module doc for the sizing.
pub(crate) const RESERVE_FDS: u64 = 128;

/// Tables smaller than this get no guard at all ([`table`] returns `None`).
/// See the module doc for why a small table fails open.
pub(crate) const MIN_TABLE_FDS: u64 = 512;

/// Live `wl_buffer`s a client may hold before pressure starts refusing its
/// creations. Only ever enforced while [`Table::pressured`] holds; the
/// per-connection 512 cap applies regardless. See the module doc.
pub(crate) const PRESSURE_GRACE_BUFFERS: u32 = 128;

/// Live `wl_shm_pool`s a client may hold before pressure starts refusing
/// its creations. Same conditional shape as [`PRESSURE_GRACE_BUFFERS`].
pub(crate) const PRESSURE_GRACE_POOLS: u32 = 64;

/// One observation of the process fd table: how many fds are open against
/// how many the kernel allows this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Table {
    /// Open fds counted in `/proc/self/fd`, including the counting dir fd
    /// itself (off by one toward pressure; the reserve absorbs it).
    pub used: u64,
    /// The `RLIMIT_NOFILE` soft limit: the table size pressure is measured
    /// against. Always finite and `>= MIN_TABLE_FDS` -- [`table`] returns
    /// `None` otherwise.
    pub soft: u64,
}

impl Table {
    /// Fds still allocatable before the table is full. Saturating: a count
    /// past the soft limit (a shrunken limit, a racing close) reads as
    /// zero free, never as a wrap into plenty.
    pub(crate) fn free(&self) -> u64 {
        self.soft.saturating_sub(self.used)
    }

    /// Whether the table is pressured: fewer than [`RESERVE_FDS`] free.
    /// Total on any input (hand-built tables included): a small `soft`
    /// reads as calm here, matching [`table`]'s `None` for the same table.
    pub(crate) fn pressured(&self) -> bool {
        self.soft >= MIN_TABLE_FDS && self.free() < RESERVE_FDS
    }
}

/// Observes the process fd table, or `None` when there is nothing to
/// enforce (small or infinite table) or nothing observable (any failure).
/// Fails open by construction: every enforcement site admits on `None`.
pub(crate) fn table() -> Option<Table> {
    let soft = soft_limit()?;
    if soft < MIN_TABLE_FDS {
        return None;
    }
    let used = std::fs::read_dir("/proc/self/fd").ok()?.count() as u64;
    Some(Table { used, soft })
}

/// The `RLIMIT_NOFILE` soft limit, or `None` when it is no table to measure
/// against (call failed, or no limit at all).
fn soft_limit() -> Option<u64> {
    // SAFETY: `getrlimit` writes exactly one `struct rlimit` through a
    // live pointer to one, and returns nonzero on failure without touching
    // it (in which case the zeroed value is never read).
    let mut limits: libc::rlimit = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limits) } != 0 {
        return None;
    }
    if limits.rlim_cur == libc::RLIM_INFINITY {
        return None;
    }
    Some(limits.rlim_cur as u64)
}

#[cfg(test)]
mod tests;
