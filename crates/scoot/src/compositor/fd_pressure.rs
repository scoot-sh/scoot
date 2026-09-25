//! Compositor-wide file-descriptor pressure: the global ceiling.
//!
//! Every other bound in this compositor is per connection (512 fds of every
//! kind a client hands over and scoot keeps -- shm pools, dma-buf planes,
//! syncobj timelines, counted until they really close, see `client_fds.rs`
//! -- plus, on objects: 512 live `wl_buffer`s, 128 live pools, 32 dma-buf
//! planes added to params objects not yet created, 8 binds, 16 capture
//! frames, 64 IPC slots, and where explicit sync is offered 128 retained
//! syncobj timelines and 64 outstanding acquire waits -- see
//! `dmabuf/pending_planes.rs` and `drm_syncobj.rs`), while the fd table they
//! all draw from is process-global (`RLIMIT_NOFILE` 1024 on the dev VM). Two
//! connections inside every per-connection bound can hold more than the
//! table with nothing tripped -- the residual
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
//! - **Arrival guards**: every request that hands scoot an fd it keeps
//!   (`wl_shm.create_pool`, `zwp_linux_buffer_params_v1.add`,
//!   `import_timeline`) is refused, with the same error its per-client bound
//!   would post, when the client already has scoot keep more than a
//!   per-client *grace* of fds (`client_fds::PRESSURE_GRACE_FDS`, 128, every
//!   kind together) while the table is pressured. The acquire-wait bound
//!   does the same for its own eventfds (`drm_syncobj/acquire.rs`, grace 16),
//!   which are scoot's, not the client's, and so not in that ledger. The
//!   grace is what makes this a ceiling rather than a lottery: a bar holding
//!   2 pools is never refused for another client's greed, only a client
//!   already holding past-grace is ever killed, and a killed client's
//!   disconnect frees what it held, so the pressure it caused lifts with it.
//!
//! The graces used to be per object kind (128 buffers, 64 pools, 8 pending
//! planes, 32 timelines). They counted objects, so they could neither see
//! the fd a destroyed object left behind (a buffer a surface still has
//! committed keeps its pool's fd after the buffer and the pool are both
//! gone) nor tell a four-plane dma-buf from a single-pixel buffer that holds
//! none. The ledger counts the fds themselves, so the one grace is checked
//! against what a client really makes this process hold. Creating a buffer
//! hands scoot no fd (a shm buffer shares its pool's; a dma-buf's planes
//! arrived with their `add`s), so buffer creation is no longer a pressure
//! site at all.
//!
//! What is deliberately *not* here: any creation-time wait, queue or silent
//! ignore (no protocol channel carries "retry later" on these interfaces,
//! and a silent ignore leaves the uninitialized object that panics the
//! compositor -- the argument `dispatch.rs` already makes), any
//! connection-count cap (any usable count admits the killing pair; see the
//! verdict above), and any kill of an idle holder: pressure refuses a
//! client's next arrival, so a client that is past its grace and then stops
//! sending keeps what it has until it leaves, and newcomers are shed
//! meanwhile. That shape needs two connections now (below).
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
//! - One connection at every bound at once, on the tier with the most
//!   (`--tty` GPU scanout with explicit sync): 512 client fds + 64
//!   acquire-wait eventfds + 1 socket = **577**, over a baseline measured at
//!   **43 fds** idle there (dev VM, `--tty --renderer gles`, 2026-09-24):
//!   **620**, 276 below the 896 line (measured live at the bound with a
//!   three-plane dma-buf client: 557). On the default pixman tier it is 513
//!   over 14. The 512 includes the copy the dev VM's software GLES renderer
//!   keeps of each imported plane (`dmabuf/renderer_copies.rs`); copies of
//!   planes that closed since the last cache drain can add one round of the
//!   client's buffers on top, 256 at most (876). So through everything
//!   scoot counts, one connection cannot trip the reserve on its own on any
//!   tier, and outside that drain window there is room for a normal session
//!   beside it (620 + ~150 = 770). Before the fd ledger the same
//!   sum on the GPU tier was ~865 + 43 = 908, past the line, and a
//!   multi-plane dma-buf or a destroyed-but-committed buffer was not in it at
//!   all. This holds on tables of 1024 fds and up; below that the line (the
//!   table minus [`RESERVE_FDS`]) comes down with the table while the
//!   per-client bound does not, so on a 512-fd table (the smallest guarded,
//!   [`MIN_TABLE_FDS`]) one connection can still reach it.
//! - Two such connections do exceed the table, which is what the arrival
//!   guards are for: the second is past its 128-fd grace long before the
//!   line, and its next arrival there is refused.
//!
//! ## What is not counted
//!
//! Below scoot, in wayland-backend, two per-connection queues hold fds that
//! no scoot code sees:
//!
//! - **Received fds** (`docs/backlog/core/wayland-backend-fd-queue.md`):
//!   fds a client sends alongside a request whose signature has no fd
//!   argument stay queued for the connection's life. Review of PR #236
//!   measured one client taking scoot from 18 to 999 fds this way on the
//!   default headless tier, newcomers shed, and the client never killed.
//!   Unbounded; that ticket's.
//! - **Outgoing fds**: an event carrying an fd (a keymap, a dma-buf format
//!   table, a selection `send`) is written into the client's outgoing buffer
//!   with a duplicate of the fd, which closes once the buffer is flushed to
//!   the socket. A client that stops reading keeps those duplicates here
//!   until the buffer is full, and is then disconnected (wayland-backend
//!   caps the buffer at 4096 bytes and kills the client past it). The
//!   smallest such event is 12-16 bytes, so that is at most a few hundred
//!   fds, and only for a client that has first filled its socket's kernel
//!   buffer. Reasoned from wayland-backend 0.3.17's source, not measured.
//!   On top of the 620 above that could take one non-reading connection on
//!   the dev VM's GPU tier to the line, but only transiently: it is
//!   disconnected once its buffer fills.
//!
//! [`RESERVE_FDS`] is 128: shed/refuse once fewer than 128 fds stand free
//! (used past 896 of 1024). That is ~6x above the reasoned login storm. What
//! the 128 is *for* is headroom once the line is crossed: scoot's own
//! transient fds (an accepted socket before it is shed, a selection pipe, an
//! acquire wait, a screenshot) and the arrivals the graces still admit
//! (below). It is not room for a greedy connection: one of those can hold
//! 512 client fds and more, and what stops it is the per-client bound and
//! the grace, not the reserve. (An earlier version of this paragraph said
//! the reserve left room for a whole greedy connection's burst; it never
//! did.)
//!
//! [`MIN_TABLE_FDS`] is 512: below it the guard stays off entirely
//! ([`table`] returns `None`, every site fails open) and the `EMFILE` shed
//! is the only backstop. A table that small cannot tell pressure from a
//! busy session -- enforcing a 128 reserve on a 256-fd table would shed a
//! normal login storm -- so the honest answer is no guard rather than a
//! hair-trigger one.
//!
//! The grace (128 fds, every kind together; see `client_fds.rs` for how it
//! compares to the heaviest reasoned legitimate clients) bites only
//! *during* genuine pressure, which a legitimate session never produces
//! (see above): holding past-grace while the table is 7/8 full means
//! contributing to the pressure, which is what justifies the kill. A
//! connection at the most the graces let through under pressure holds
//! 128 + 16 + 17 + 1 = 162 fds on the GPU tier: 128 client fds plus the
//! [`SWEEP_MARGIN`](crate::compositor::client_fds::SWEEP_MARGIN) the ledger
//! admits between pressure checks, 17 acquire-wait eventfds (`live > 16` is
//! the refusal), and a socket. Two such connections are 367 with the idle
//! baseline, far below the line. But that does not generalise to "pressure
//! always needs someone past grace": six connections at the grace reach
//! 6 x 162 + 43 = 1015, past 896 with nobody past any grace, and then
//! newcomers are shed and no one is refused. That is connection
//! multiplication, the residual
//! `docs/backlog/resolved/wayland-connection-cap-done.md` already accepted
//! (any usable per-connection budget times enough connections fills the
//! table), not a hole this ceiling can close. It is a little easier to reach
//! than before the fd ledger: the pending-plane grace of 8 is gone, so a
//! client holding 32 pending planes and nothing else is under the 128-fd
//! grace and is not refused under pressure. Review of PR #239 measured 17
//! such connections all held and a newcomer shed at 584 fds under a 700-fd
//! table, with nobody killed. The grace attributes pressure to *heavy*
//! clients; many light ones are the connection-count problem.
//!
//! ## Observation cost and disciplines
//!
//! [`table`] costs one `getrlimit` plus one `/proc/self/fd` readdir --
//! ~8us per call on the dev VM (debug build, 2000-call sample; release is
//! faster), no steady-state cost anywhere: the accept sites run it once per
//! *connection* (not per frame or request), and the arrival sites only
//! once a client is already past its grace, and then at most once per
//! [`SWEEP_MARGIN`](crate::compositor::client_fds::SWEEP_MARGIN) arrivals
//! (a map lookup short-circuits everything under it).
//!
//! `table()` allocates (`read_dir`), so the fork-child discipline from
//! `ipc::accept` applies: never call it from `drain`, `shed_one`,
//! `classify`, or anything a forked exhaustion-test child executes. Every
//! current call site (the two accept callbacks, the arrival guards and the
//! acquire-wait bound) runs on the loop thread, never in a forked child.
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
