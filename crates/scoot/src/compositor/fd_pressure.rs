//! Compositor-wide file-descriptor pressure: the global ceiling.
//!
//! Every other bound in this compositor is per connection (512 fds of every
//! kind a client hands over and scoot keeps -- shm pools, dma-buf planes,
//! syncobj timelines, counted until they really close, see `client_fds.rs`
//! -- plus, on objects: 512 live `wl_buffer`s, 128 live pools, 32 dma-buf
//! planes added to params objects not yet created, 8 binds, 16 capture
//! frames, 64 IPC slots, and where explicit sync is offered 128 retained
//! syncobj timelines and 64 outstanding acquire waits -- see
//! `dmabuf/pending_planes.rs` and `drm_syncobj.rs`; and below scoot, the
//! received fds no request claimed, capped in the wayland-backend fork at
//! [`backend_queued_fds`] of the table), while the fd table they all draw
//! from is process-global: the soft `RLIMIT_NOFILE`, which scoot raises at
//! startup to its hard limit capped at 65536 (`nofile.rs`; 65536 on the dev
//! VM, whose hard limit is 524288), and which stays 1024 where the hard limit
//! is 1024 (a container, say). Every line here is measured against the
//! actual limit ([`table`] reads it); the figures below are given for both
//! tables. On the 1024-fd table, two connections inside every per-connection
//! bound can hold more than the table with nothing tripped -- the residual
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
//! - Adding what wayland-backend holds for that connection below scoot (next
//!   section): its received-fd queue at the cap, [`backend_queued_fds`] at
//!   rest, and up to 30 more for a moment inside the read that takes it past
//!   (the next check disconnects it).
//!   - **Raised table (65536, line 65408)**: the cap is 1024. Steady state
//!     577 + 1024 = 1601 at rest, 1631 inside that read, **1674** with the
//!     GPU tier's baseline; the drain window adds 256 (1930). Nowhere near
//!     the line, and 63 such connections would still fit under it.
//!   - **1024-fd table (line 896)**: the cap is 128. Steady state 705 at rest
//!     and 735 inside the read, **748** and **778** with the baseline, 148
//!     and 118 below the line. The drain window is where it stops fitting:
//!     876 + 128 = 1004 at rest, past the line (newcomers are shed for that
//!     moment) but inside the table; and 1034 inside the read, past the
//!     table. What that last instant would do is reasoned, not measured: the
//!     kernel installs the read's fds only up to the table and closes the
//!     rest, so the table is full for the remainder of that one read's
//!     requests, all from the connection that is about to be disconnected
//!     (it is past the queue cap, or its fd-carrying requests meet its full
//!     512 and are refused). A request of its that makes scoot send an fd to
//!     *another* client in that window (a selection `send`) fails to
//!     duplicate the fd, and wayland-backend disconnects that other client
//!     for it: the same harm any full table does, and one the
//!     connection-count residual below already reaches with several
//!     connections. Here it needs one connection at its 512 bound, in the
//!     software-GLES drain window, with 128 parked, all in the same instant,
//!     on a machine whose hard limit kept the table at 1024.
//!
//!   `tests.rs` derives the steady-state figures on both tables from the
//!   ledger's admission rule and the queue terms, which
//!   `tests/backend_queue.rs` pins against the real backend.
//!
//! ## Below scoot, in wayland-backend
//!
//! Two per-connection queues there hold fds that no scoot code sees, so no
//! ledger counts them and the grace cannot attribute them:
//!
//! - **Received fds**: every fd arrives attached to a request's bytes and is
//!   queued until a request with an fd argument takes it, so fds a client
//!   attaches to fd-less requests are never taken. Released wayland-backend
//!   0.3.17 kept them for the connection's life; review of PR #236 measured
//!   one idle client taking scoot from 18 to 999 fds that way, newcomers and
//!   `scootctl` shed, the client never killed. scoot now builds against a
//!   scoot-sh fork (`docs/forks.md`) that disconnects a client leaving more
//!   than its cap unclaimed at a point where every complete request has been
//!   parsed, with `wl_display.error` `invalid_method` ("too many file
//!   descriptors queued"), closing them. The cap is one eighth of the soft
//!   limit read when the client connects, clamped to 128..=1024
//!   ([`backend_queued_fds`] restates it): **1024** on the raised table,
//!   which is libwayland-server's own bound (its `fds_in` ring holds 4096
//!   bytes of fds by default), and **128** on a 1024-fd table. One read adds
//!   at most 30 on top. So a connection holds at most the cap at rest and
//!   the cap + 30 for a moment, the figures above
//!   (`docs/backlog/resolved/wayland-backend-fd-queue-done.md`).
//!
//!   The check runs before each read, so fds whose requests are still in
//!   flight count, and well-behaved clients do run ahead: a flush carrying
//!   more than 28 fds sends them 28 per `sendmsg` with one byte each, ahead
//!   of the bytes. A client on `wayland-client`'s pure-Rust backend does that
//!   for every flush; a libwayland client does it once its socket has filled
//!   and its unbounded buffers have grown (review of PR #241 measured a stock
//!   libwayland 1.26 client stalled behind a stopped compositor disconnected
//!   at 140 fds under the old fixed 128). At the 1024 cap every client a
//!   libwayland compositor serves is served: `tests/backend_queue_client.rs`
//!   pins the backpressure shape (the cap served, one more disconnected) and
//!   the Rust client's largest one-flush batch (1036), and on the dev VM the
//!   same libwayland client is served at 160, 600 and 1000. On a 1024-fd
//!   table it is still disconnected past about 128.
//! - **Outgoing fds**: an event carrying an fd (a keymap, a dma-buf format
//!   table, a selection `send`) is written into the client's outgoing buffer
//!   with a duplicate of the fd, which closes once the buffer is flushed to
//!   the socket. A client that stops reading keeps those duplicates here
//!   until the buffer is full, and is then disconnected (wayland-backend
//!   caps the buffer at 4096 bytes and kills the client past it). The
//!   smallest such event is 12-16 bytes, so that is at most a few hundred
//!   fds, and only for a client that has first filled its socket's kernel
//!   buffer. Reasoned from wayland-backend 0.3.17's source (unchanged in the
//!   fork), not measured. On top of the 620 above that could take one
//!   non-reading connection on the dev VM's GPU tier to the line, but only
//!   transiently: it is disconnected once its buffer fills.
//!
//! [`RESERVE_FDS`] is 128: shed/refuse once fewer than 128 fds stand free
//! (used past 65408 of the raised 65536, or past 896 of a 1024-fd table; on
//! the small table that is ~6x above the reasoned login storm). A fixed
//! reserve rather than a fraction of the table on purpose: what the 128 is
//! *for* is headroom once the line is crossed, the same on any table: scoot's own
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
//! (see above): holding past-grace while the table is nearly full means
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
//! The wayland-backend queue cap reshapes that residual: fds parked there
//! need no object at all, and the grace never sees them, so each idle
//! connection can hold its cap plus its socket. Measured on the default tier
//! (headless pixman, idle at 18; `~/evidence/fdq/runs/`):
//!
//! - **Raised table**: 1025 per connection. 8 such connections held 8218
//!   fds and 63 held 64593, everyone served both times; 64 filled the 65536
//!   table (newcomers dropped, `scootctl` reset). Nobody is disconnected.
//! - **1024-fd table** (the fixed-128 build, which is what a 1024 hard limit
//!   gives): 129 per connection. Six held 792 with everyone served, seven 921,
//!   past the line (newcomers shed, `scootctl` refused), and eight filled the
//!   table.
//!
//! Before the cap, one connection could do either. That residual, and what
//! it costs `scootctl`, is
//! `docs/backlog/core/pressure-many-light-connections.md`.
//!
//! ## Observation cost and disciplines
//!
//! [`table`] costs one `getrlimit` plus one `/proc/self/fd` readdir, which
//! is linear in the open fds: ~8us per call on the dev VM at a session's few
//! dozen (debug build, 2000-call sample; release is faster), and measured
//! for the readdir alone at 3.6us for 20 open fds, 72us for 1000, 634us for
//! 8000 and ~8ms for 65000 (`~/evidence/fdq/runs/readdir-cost.txt`). So on
//! the raised table an observation stays cheap in any real session but costs
//! ~8ms per accepted connection once someone has filled the table, which is
//! the price of the raise's headroom. No steady-state cost anywhere: the
//! accept sites run it once per
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

use std::cell::Cell;
use std::time::{Duration, Instant};

/// How many received fds wayland-backend lets one client leave unclaimed on a
/// table of `soft` fds: one eighth of it, clamped to 128..=1024. This mirrors
/// the scoot-sh fork's `max_queued_fds` (crate-private there, read when each
/// client is created) so the arithmetic below and the startup log can name
/// it; `tests/backend_queue.rs` pins it against the real backend.
pub(crate) fn backend_queued_fds(soft: u64) -> u64 {
    (soft / 8).clamp(128, 1024)
}

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

/// The process fd table as the enforcement sites see it: a cached
/// [`observe`], refreshed at most once per [`reading_lifetime`], plus every
/// fd [`note_opened`] has counted since. `None` when there is nothing to
/// enforce (small or infinite table) or nothing observable (any failure).
/// Fails open by construction: every enforcement site admits on `None`.
///
/// Why cached: an observation is a readdir of `/proc/self/fd`, linear in the
/// open fds (~7 ms at 60000), and the sites are client-triggered: the
/// Wayland accept callback drains its whole backlog (up to 4096
/// connections) in one call, the IPC accept runs per connection, and a
/// client past its grace reaches the creation guards on every request.
/// Uncached, review of PR #241 measured a 4000-connection storm against
/// 58000 parked fds freezing the compositor for 35.6 s. Cached, a burst
/// costs one readdir, and readdirs cost at most ~1/20 of loop time however
/// large the table (see [`reading_lifetime`]).
///
/// The cache is per thread: every site runs on the event-loop thread, and
/// a test's `State` gets a reading of its own. What it can miss is fds
/// opened within a reading's lifetime by anything other than the accepts
/// (which [`note_opened`] counts): client fds arriving on requests, and
/// scoot's own. For at most one lifetime the reading is low by those; the
/// reserve absorbs that, and the `EMFILE` shed still catches real
/// exhaustion underneath. Fds closed within a lifetime make it read high
/// until the next refresh: towards shedding, never away from it.
pub(crate) fn table() -> Option<Table> {
    let now = Instant::now();
    GAUGE.with(|gauge| {
        if let Some(cached) = gauge.get().filter(|cached| cached.fresh_at(now)) {
            return cached.table;
        }
        let started = Instant::now();
        let table = observe();
        let cost = started.elapsed();
        OBSERVATIONS.with(|count| count.set(count.get() + 1));
        gauge.set(Some(Reading {
            table,
            taken: now,
            lifetime: reading_lifetime(cost),
        }));
        table
    })
}

/// Counts `count` fds just opened on the loop thread (an accepted
/// connection) against the cached reading, so a burst of accepts inside one
/// reading's lifetime still sees the table fill: the 4000th connection of a
/// storm is judged against a count that includes the 3999 before it.
pub(crate) fn note_opened(count: u64) {
    GAUGE.with(|gauge| {
        if let Some(mut cached) = gauge.get() {
            if let Some(table) = cached.table.as_mut() {
                table.used = table.used.saturating_add(count);
            }
            gauge.set(Some(cached));
        }
    });
}

/// How long a reading that took `cost` to observe is reused: at least
/// [`MIN_READING_LIFETIME`], and at least 20 times its cost, so observing
/// costs at most ~5% of loop time on any table (a 7 ms readdir at 60000 open
/// fds is reused for 140 ms). On an ordinary session's few hundred fds a
/// readdir costs microseconds, and the reading is at most 1 ms old.
pub(crate) fn reading_lifetime(cost: Duration) -> Duration {
    cost.saturating_mul(20).max(MIN_READING_LIFETIME)
}

/// The shortest a reading is reused for; see [`reading_lifetime`].
pub(crate) const MIN_READING_LIFETIME: Duration = Duration::from_millis(1);

/// One cached observation.
#[derive(Debug, Clone, Copy)]
struct Reading {
    table: Option<Table>,
    taken: Instant,
    lifetime: Duration,
}

impl Reading {
    fn fresh_at(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.taken) < self.lifetime
    }
}

thread_local! {
    static GAUGE: Cell<Option<Reading>> = const { Cell::new(None) };
    /// How many real observations (readdirs) [`table`] has made on this
    /// thread; read by the tests that pin the per-burst bound.
    static OBSERVATIONS: Cell<u64> = const { Cell::new(0) };
}

/// Real observations [`table`] has made on this thread so far.
#[cfg(test)]
pub(crate) fn observations() -> u64 {
    OBSERVATIONS.with(Cell::get)
}

/// Replaces this thread's cached reading with `table`, reused for
/// `lifetime`: lets a test put the gauge in a known state (calm, one fd from
/// the line) without filling the process's real fd table.
#[cfg(test)]
pub(crate) fn pin_reading(table: Option<Table>, lifetime: Duration) {
    GAUGE.with(|gauge| {
        gauge.set(Some(Reading {
            table,
            taken: Instant::now(),
            lifetime,
        }));
    });
}

/// Drops this thread's cached reading, so the next [`table`] observes.
#[cfg(test)]
pub(crate) fn forget_reading() {
    GAUGE.with(|gauge| gauge.set(None));
}

/// Observes the process fd table now: one `getrlimit` and one readdir of
/// `/proc/self/fd`. Only [`table`] calls this.
fn observe() -> Option<Table> {
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
