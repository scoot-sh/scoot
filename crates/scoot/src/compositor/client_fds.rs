//! Every file descriptor a Wayland client hands this compositor that the
//! compositor keeps, counted per client until the fd really closes: the one
//! ledger behind [`MAX_FDS_PER_CLIENT`], the timeline cap, and fd pressure's
//! per-client attribution.
//!
//! ## What is counted
//!
//! Three requests hand scoot an fd it keeps past the request:
//!
//! - **`wl_shm.create_pool`** ([`Kind::Pool`]). Smithay's `InnerPool` owns
//!   the request's `OwnedFd` and the pool's mapping, and every `wl_buffer`
//!   made from the pool holds an `Arc` of it. One pool is one fd, however many
//!   buffers share it.
//! - **`zwp_linux_buffer_params_v1.add`** ([`Kind::Plane`]). The params
//!   object holds the plane's fd until `create`/`create_immed` moves it into
//!   a `Dmabuf` (an `Arc<OwnedFd>` per plane), which the `wl_buffer` holds.
//!   One `add` is one fd, so a four-plane buffer is four.
//! - **`wp_linux_drm_syncobj_manager_v1.import_timeline`**
//!   ([`Kind::Timeline`], only where explicit sync is offered). Smithay's
//!   `DrmTimelineInner` owns the fd, and every sync point on the timeline
//!   holds an `Arc` of it.
//!
//! In all three the fd is the request's own `OwnedFd`, moved unchanged, never
//! duplicated (checked in the pinned fork: no `try_clone` or `dup` on any of
//! these paths, Smithay's or scoot's). So "what the client made scoot keep"
//! and "which fd numbers from that client are still open" are the same thing.
//!
//! ## Why objects cannot be counted instead
//!
//! Each of those fds outlives the object it arrived on, and not only by
//! protocol design:
//!
//! - A pool's fd lives while any buffer made from it does (the protocol
//!   requires that), and a buffer's lives while anything holds its handle.
//!   The renderer's copy of a surface's state holds the handle of the buffer
//!   last committed there, after the client has destroyed both the
//!   `wl_buffer` and the pool. So "create a pool and a buffer, attach and
//!   commit on a fresh surface, destroy both" keeps one fd and one mapping per
//!   surface with no pool and no buffer object left to count. Measured before
//!   this ledger: 200 surfaces, 200 fds held, 0 buffers and 0 pools counted
//!   (`docs/backlog/resolved/buffer-fds-past-their-object-done.md`).
//! - A dma-buf's planes live in the `Dmabuf` the same way.
//! - A timeline's fd lives while any sync point on it does, and the protocol
//!   says destroying a timeline does not unset its points. Review of PR #233
//!   measured 440 surfaces holding 927 fds with no timeline object left
//!   (`docs/backlog/resolved/client-held-fd-bound-done.md`).
//!
//! scoot cannot see the retention sites (Smithay's surface caches, its
//! renderer state, the `Pool`/`DrmTimeline` `Arc`s are all private), and the
//! Smithay fork is approved for one handle-leak `Drop` only. What scoot can
//! see is the fd itself.
//!
//! ## How a record is found dead
//!
//! Each arrival is recorded by fd number against its client, before the
//! request is delegated. A record is dead once its fd has closed, and the
//! ledger learns that two ways, both exact:
//!
//! - **The number arrives again** ([`ClientFds::fd_arrived`]). The kernel
//!   hands out only closed numbers, so any fd number reaching scoot through
//!   one of the three requests proves the record on it dead, whoever owned
//!   it.
//! - **A sweep checks it** (`liveness.rs`). A pool or plane record carries
//!   the identity (`st_dev`, `st_ino`) its fd had on arrival, and is live
//!   only while an `fstat` of the number still gives that identity. So a
//!   number that closed and was reused by anything else scoot opens (a new
//!   client's socket, an eventfd, a DRM fd) reads as dead. A memfd, a shm
//!   file and a dma-buf each have their own inode. A syncobj does not: it is
//!   made by `anon_inode_getfile`, whose files (eventfds among them) all
//!   share the kernel's one anonymous inode, so a timeline record is checked
//!   by the link name instead (`readlink /proc/self/fd/N` is
//!   `anon_inode:syncobj_file`), as before.
//!
//! A pool's fd closes a moment after the pool itself is dropped: Smithay
//! drops `InnerPool` on its own "Shm dropping thread", because closing can
//! take milliseconds. A sweep in that moment counts the fd, which it still
//! is. Only a client at a bound ever sees that, and only for the pools it
//! has just released.
//!
//! **Mappings.** `InnerPool` declares its mapping before its fd, so dropping
//! it unmaps first and closes second. An open pool fd is therefore an upper
//! bound on its mapping, and the pool records bound the shm mappings a
//! client makes scoot keep, destroyed-but-committed ones included, at up to
//! 512 MiB each (the per-pool cap in `dispatch.rs`). Before this ledger that
//! count was bounded only while the buffers were live objects.
//!
//! **Renderer copies.** One more fd can follow a plane: a GLES renderer on
//! Mesa's software rasterizer keeps its own duplicate of every plane it
//! imports, for as long as its texture cache holds the import (a hardware
//! driver is expected to keep none; that module says how far that is
//! checked). That duplicate is not the client's fd, so no arrival brings
//! it. Instead a plane is *admitted at the weight it will cost*: 1, plus the
//! copies each GLES backend will make when it is imported, which
//! `dmabuf/renderer_copies.rs` learns once per session and assumes is 1 until
//! it has. Every bound here reads the weighted sum, and since the copies are
//! inside the weight the arrival was admitted at, an import can never take a
//! client past a bound after the fact. (It used to: the copies were added at
//! import, and review of PR #239 took a client to 540 against 512 by adding
//! planes near the bound and importing them.) Once the session knows the
//! real number, an import lowers its planes to it
//! ([`ClientFds::settle_copies`]); a weight is never raised after admission.
//! A copy can outlive its plane until the renderer's cache is next drained;
//! that module states the window.
//!
//! ## The numbers
//!
//! [`MAX_FDS_PER_CLIENT`] is **512**, across all three kinds. The heaviest
//! legitimate single clients are reasoned, not measured (nothing on the dev
//! VM makes a dma-buf; see `dmabuf.rs`): a GPU browser with 20 windows at
//! triple buffering plus a few shm pools is ~80-100; a video player keeping
//! ~30 decoded two-plane frames is ~60 plus its swapchain; Xwayland, one
//! client for every X window, gives each presented window pixmap its own
//! pool or dma-buf, ~2-3 per mapped window; a Vulkan window adds 16
//! timelines. 512 is several times all of those, and the same number the
//! live-buffer cap already allowed as one-fd-per-buffer. It is generous on
//! purpose, because hitting it disconnects the client.
//!
//! [`PRESSURE_GRACE_FDS`] is **128**, what fd pressure (`fd_pressure.rs`)
//! lets a client keep before it may refuse that client's next arrival.
//!
//! What one connection can make this process hold, at every bound at once,
//! on the tier with the most (`--tty` GPU scanout, explicit sync offered):
//! 512 here (renderer copies included, and never exceeded: see
//! [`ClientFds::admit`]), plus 64 acquire-wait eventfds
//! (`drm_syncobj/acquire.rs`, scoot's own fds, bounded there), plus its
//! socket: **577**. Against the idle baseline measured there (43) that is
//! 620, 276 below the pressure line of a 1024-fd table (896); measured live
//! at the bound, 557. `fd_pressure/tests.rs` derives that figure by driving
//! this ledger's admission rule, not from the constants alone. That is the
//! steady state. The one transient on top is the renderer-copy drain window
//! (`dmabuf/renderer_copies.rs`): copies of planes that already closed, until
//! the next cache drain. Bounded by one round of the client's planes -- at
//! most 256 copies at two fds a plane, 876 in all, still under the line --
//! and in practice short: review of PR #239 saw all 240 copies of 80 released
//! buffers close within 500 ms on both GLES tiers. What scoot does not count
//! is in `fd_pressure.rs`.
//!
//! ## When the bounds are checked
//!
//! Only on an arrival, the one event that adds a record. The records
//! ([`ClientFds::held_by`]) are an upper bound on what the client really
//! holds: every fd it made scoot keep has one, and dead ones linger until
//! something notices. Refusing on that number alone would kill a legitimate
//! client whose churn left dead records, so a refusal is only ever decided
//! on a fresh sweep ([`ClientFds::admit`]), and [`SWEEP_MARGIN`] keeps those
//! sweeps off the per-request path.
//!
//! In practice dead records rarely pile up: the kernel hands out the lowest
//! free number, so a client that releases and reallocates usually gets its
//! old numbers back, and each one's arrival forgets the record on it.
//!
//! ## Bounds on the ledger itself
//!
//! Each fd number has at most one record, since a new record on a number
//! replaces the old one. So the ledger never holds more records than the fd
//! table has numbers, however many clients come and go. A disconnected
//! client's records are never swept (only an arrival sweeps, and only its
//! own client's records), so they stay until their numbers arrive again.
//! That is bounded the same way, so no disconnect hook is needed.
//!
//! ## A known over-count
//!
//! wayland-backend keeps fds that a client sends alongside a request with no
//! fd argument, for the connection's life
//! (`docs/backlog/core/wayland-backend-fd-queue.md`). A syncobj fd parked
//! there on a number some other client's dead timeline record names would
//! make that record read live, and so inflate the *other* client's count.
//! Pool and plane records are immune (their check is the file's identity,
//! which a different file does not have); timeline records are not, since
//! every syncobj shares one inode. Reasoned, not demonstrated. It can only
//! over-count, never under-count, so it opens no hole in the bound; its harm
//! is that it could push an innocent client toward a refusal, which is part
//! of that ticket.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, BorrowedFd, RawFd};

use smithay::reexports::wayland_server::backend::ClientId;

pub(crate) use liveness::Check;

mod liveness;

#[cfg(test)]
mod tests;

/// How many fds one client may have this process keep at once: shm pools,
/// dma-buf planes and syncobj timelines together, each counted until it
/// really closes. See the module doc for the number.
///
/// An arrival that would take the client's weight past this bound sweeps
/// its records first, and is refused unless the sweep leaves room for it
/// with [`SWEEP_MARGIN`] to spare (for a one-fd arrival: refused if more than
/// `512 - SWEEP_MARGIN` = 496 are still open). A plane is admitted at its
/// full weight, renderer copies included, and no weight is raised after
/// admission, so the fds a client has scoot keep never pass 512 (apart from
/// the drain window the module doc names). The refusal is the arriving
/// request's own (see [`Refusal`]), and kills only that client.
pub(crate) const MAX_FDS_PER_CLIENT: u32 = 512;

/// How many fds a client may have this process keep before fd pressure
/// (`fd_pressure.rs`) starts refusing its arrivals. Enforced only while the
/// process fd table is pressured, so a client under it is never refused for
/// another client's greed.
///
/// 128 is past what any reasoned single legitimate client keeps (see the
/// module doc), so while the table is pressured a client past it is a
/// contributor. It replaces the per-kind graces fd pressure used to read
/// (128 buffers, 64 pools, 8 pending planes, 32 timelines): those counted
/// objects, and so could neither see the fds a destroyed object left behind
/// nor tell a four-plane buffer from a single-pixel one.
pub(crate) const PRESSURE_GRACE_FDS: u32 = 128;

/// How far below a bound a sweep must bring a client for the arrival that
/// triggered it to be admitted. After such a sweep the client can make this
/// many more arrivals before the next one.
///
/// This is what keeps sweeps off the per-request path. Without it, a client
/// that really holds one fewer than a bound and churns would trigger a
/// sweep, one syscall or two per record, on every arrival, paced by the
/// client. With it, sweeps are at most one per 16 arrivals. Under fd pressure
/// the margin is admission slack instead: after any check (a sweep that
/// admits, or a calm table observation) a client may make up to 16 more
/// arrivals before the next, so it can go 16 past what it held when last
/// checked.
pub(crate) const SWEEP_MARGIN: u32 = 16;

/// Which request handed scoot a recorded fd.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// `wl_shm.create_pool`.
    Pool,
    /// `zwp_linux_buffer_params_v1.add`.
    Plane,
    /// `wp_linux_drm_syncobj_manager_v1.import_timeline`.
    Timeline,
}

/// The bounds [`ClientFds::admit`] enforces. Production passes [`LIMITS`];
/// a parameter so that the decisions are unit-testable at small numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// Every kind together: [`MAX_FDS_PER_CLIENT`].
    pub total: u32,
    /// Timelines alone: `drm_syncobj::MAX_TIMELINES_PER_CLIENT`.
    pub timelines: u32,
    /// The pressure grace on the total: [`PRESSURE_GRACE_FDS`].
    pub grace: u32,
}

/// The bounds production enforces.
pub(crate) const LIMITS: Limits = Limits {
    total: MAX_FDS_PER_CLIENT,
    timelines: super::drm_syncobj::MAX_TIMELINES_PER_CLIENT,
    grace: PRESSURE_GRACE_FDS,
};

/// Why [`ClientFds::admit`] refused an arrival, with what the fresh sweep
/// found the client still holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// At [`MAX_FDS_PER_CLIENT`], and a sweep could not free
    /// [`SWEEP_MARGIN`]. `held` is every kind.
    Total { held: u32 },
    /// A timeline import at the timeline cap, and a sweep could not free
    /// [`SWEEP_MARGIN`]. `held` is timelines only.
    Timelines { held: u32 },
    /// Past [`PRESSURE_GRACE_FDS`] while the fd table is pressured. `held`
    /// is every kind.
    Pressure { held: u32 },
}

impl Refusal {
    /// The protocol error's message, naming the bound that said no. `refused`
    /// says what was refused, e.g. "wl_shm pool refused". Allocates, which is
    /// fine: this runs only on the path that is about to disconnect a client.
    pub(crate) fn message(self, refused: &str) -> String {
        match self {
            Refusal::Total { held } => format!(
                "{refused}: this compositor still holds {held} file descriptors for this client \
                 (the shm pools, dma-buf planes and syncobj timelines it handed over, counted \
                 until they close even after their objects are destroyed, and any copies the \
                 renderer keeps of an imported plane), and the maximum is {MAX_FDS_PER_CLIENT}"
            ),
            Refusal::Timelines { held } => format!(
                "{refused}: this compositor still holds {held} of this client's imported \
                 timelines (live ones, and destroyed ones its sync points still reference), and \
                 the maximum is {}",
                super::drm_syncobj::MAX_TIMELINES_PER_CLIENT
            ),
            Refusal::Pressure { held } => format!(
                "{refused}: compositor-wide file-descriptor pressure, and this compositor still \
                 holds {held} file descriptors for this client, more than the \
                 {PRESSURE_GRACE_FDS}-fd pressure grace"
            ),
        }
    }
}

/// The ledger. See the module doc.
#[derive(Debug, Default)]
pub struct ClientFds {
    /// Which client handed scoot the fd recorded on each number. At most one
    /// record per number, so this is bounded by the fd table's size.
    owner: HashMap<RawFd, ClientId>,
    /// Each client's records, as a list for its own sweep.
    per_client: HashMap<ClientId, Held>,
    /// Record lists of entries that emptied, reused by the next entry made
    /// (see [`ClientFds::release_entry`]).
    spare: Vec<Vec<Record>>,
}

/// How many emptied record lists [`ClientFds`] keeps for reuse.
const SPARE_LISTS: usize = 8;

/// The capacity an emptied record list is shrunk to before it is kept.
const SPARE_LIST_CAPACITY: usize = 64;

/// One recorded fd.
#[derive(Debug, Clone, Copy)]
struct Record {
    fd: RawFd,
    kind: Kind,
    /// How a sweep tells whether the fd on this number is still this one.
    check: Check,
    /// How many fds this record stands for: 1, plus the copies a renderer
    /// is expected to keep of a plane's fd once it is imported, charged when
    /// the plane is admitted and only ever lowered after
    /// ([`ClientFds::settle_copies`]). Always at least 1.
    weight: u8,
}

/// One client's records.
#[derive(Debug, Default)]
struct Held {
    /// Unordered, and never longer than [`Limits::total`]: every record
    /// weighs at least 1, and the weight never passes that bound (an arrival
    /// that would take it past sweeps first or is refused, and a weight is
    /// never raised after admission).
    records: Vec<Record>,
    /// How many of `records` are [`Kind::Timeline`].
    timelines: u32,
    /// The sum of `records`' weights: the fds this client has this process
    /// keep, as far as the ledger knows. What the total bound and the
    /// pressure grace read.
    weight: u32,
    /// The pressure-check amortization: no pressure check until the records
    /// reach this. Set to live plus [`SWEEP_MARGIN`] by every sweep, to
    /// records plus [`SWEEP_MARGIN`] by a calm observation, and 0 (meaning
    /// due) before the first.
    sweep_at: u32,
}

impl Held {
    /// Takes `record`'s weight (and its timeline) off the sums, for a record
    /// being dropped.
    fn forget(&mut self, record: &Record) {
        self.weight = self.weight.saturating_sub(u32::from(record.weight));
        if record.kind == Kind::Timeline {
            self.timelines = self.timelines.saturating_sub(1);
        }
    }
}

impl ClientFds {
    /// How many fds `client` has this process keep, every kind, as recorded:
    /// the weighted sum, so an imported plane a renderer keeps a copy of
    /// counts both. An upper bound on the real number, apart from the one
    /// window the module doc names (a renderer's copy outliving its plane
    /// until the next cache drain).
    pub(crate) fn held_by(&self, client: &ClientId) -> u32 {
        self.per_client.get(client).map_or(0, |held| held.weight)
    }

    /// How many timeline fds are recorded for `client`. An upper bound, like
    /// [`Self::held_by`].
    pub(crate) fn timelines_held_by(&self, client: &ClientId) -> u32 {
        self.per_client.get(client).map_or(0, |held| held.timelines)
    }

    /// Whether a pressure check of `client` is due: its records have grown
    /// [`SWEEP_MARGIN`] past where its last check left them, or it was never
    /// checked. A check is a sweep, or a table observation that came back
    /// calm.
    pub(crate) fn sweep_due(&self, client: &ClientId) -> bool {
        self.per_client
            .get(client)
            .is_some_and(|held| held.weight >= held.sweep_at)
    }

    /// Decides an arrival of `kind` from `client` in production: forgets any
    /// record on `fd`'s number (it arrived again, so that one closed), then
    /// [`Self::admit`]s against [`LIMITS`] with the real liveness checks and
    /// the real fd table. Records nothing; the caller records the fd with
    /// [`Self::record_arrival`] once every other bound on the request has
    /// admitted it too.
    pub(crate) fn admit_arrival(
        &mut self,
        client: &ClientId,
        fd: RawFd,
        kind: Kind,
        weight: u8,
    ) -> Result<(), Refusal> {
        self.fd_arrived(fd);
        self.admit(client, kind, weight, LIMITS, liveness::still_held, || {
            super::fd_pressure::table().is_some_and(|table| table.pressured())
        })
    }

    /// Records `fd`, just admitted from `client` as `kind` at `weight`, with
    /// the identity check a later sweep will use. One `fstat` for a pool or a
    /// plane, none for a timeline, plus one insert in each of two maps.
    pub(crate) fn record_arrival(
        &mut self,
        client: &ClientId,
        fd: BorrowedFd<'_>,
        kind: Kind,
        weight: u8,
    ) {
        let check = liveness::capture(fd, kind);
        self.record(client, fd.as_raw_fd(), kind, check, weight);
    }

    /// Decides whether `client` may have scoot keep one more fd of `kind`,
    /// weighing `weight` (1, plus the renderer copies a plane will cost; see
    /// the module doc), against `limits`, with the grace applying only while
    /// `pressured()` says the fd table is. Records nothing.
    ///
    /// Every refusal is decided on a fresh sweep, never on the raw record
    /// count, which may include dead records:
    ///
    /// - **Caps.** An arrival that would take the weight past `limits.total`
    ///   sweeps first, and is refused if what the sweep leaves, plus its own
    ///   weight, is past `limits.total - SWEEP_MARGIN + 1`. So an admitted
    ///   arrival never takes the weight past `limits.total`, whatever it
    ///   weighs. The same for a timeline import against `limits.timelines`,
    ///   counting timelines only. Otherwise the client has at least
    ///   [`SWEEP_MARGIN`] weight to go before it can reach the bound again,
    ///   which is what amortizes the sweeps.
    /// - **Pressure grace.** If the arrival would take the weight past
    ///   `limits.grace + 1`, and only if a check is due ([`Self::sweep_due`])
    ///   or a sweep has just run for a cap, the table is observed
    ///   (`pressured`, a `/proc/self/fd` readdir in production). If it is
    ///   pressured, the client is refused if a sweep leaves it where the
    ///   arrival would still take it past `limits.grace + 1`. Either way the
    ///   next check is [`SWEEP_MARGIN`] later: a sweep that does not refuse
    ///   sets it from what the sweep left, and a calm observation from the
    ///   weight as it stands. So at most `SWEEP_MARGIN` weight passes between
    ///   checks. (With `weight` 1 both rules are exactly the ones PR #236's
    ///   timeline ledger had: refused past `cap - 16` live, and past the
    ///   grace.)
    ///
    /// `pressured` is evaluated at most once per call, and at most once per
    /// [`SWEEP_MARGIN`] weight per client. Under the grace, which is where
    /// every well-behaved client is, this is two map lookups and no syscall.
    pub(crate) fn admit(
        &mut self,
        client: &ClientId,
        kind: Kind,
        weight: u8,
        limits: Limits,
        mut still_held: impl FnMut(RawFd, Check) -> bool,
        pressured: impl FnOnce() -> bool,
    ) -> Result<(), Refusal> {
        // What the arrival adds beyond one fd: 0 for anything but a plane a
        // renderer will copy.
        let extra = u32::from(weight.max(1)) - 1;
        let mut total = self.held_by(client);
        let at_timeline_cap =
            kind == Kind::Timeline && self.timelines_held_by(client) >= limits.timelines;
        let mut fresh = false;
        if total.saturating_add(extra) >= limits.total || at_timeline_cap {
            let (live, timelines) = self.sweep(client, &mut still_held);
            total = live;
            fresh = true;
            if total.saturating_add(extra) > limits.total.saturating_sub(SWEEP_MARGIN) {
                return Err(Refusal::Total { held: total });
            }
            if kind == Kind::Timeline && timelines > limits.timelines.saturating_sub(SWEEP_MARGIN) {
                return Err(Refusal::Timelines { held: timelines });
            }
        }
        if total.saturating_add(extra) > limits.grace && (fresh || self.sweep_due(client)) {
            if !pressured() {
                // Calm: not again until another margin's worth of weight.
                self.check_again_after(client, total);
                return Ok(());
            }
            if !fresh {
                total = self.sweep(client, &mut still_held).0;
            }
            if total.saturating_add(extra) > limits.grace {
                return Err(Refusal::Pressure { held: total });
            }
        }
        Ok(())
    }

    /// Sets `client`'s next pressure check [`SWEEP_MARGIN`] records past
    /// `held`.
    fn check_again_after(&mut self, client: &ClientId, held: u32) {
        if let Some(entry) = self.per_client.get_mut(client) {
            entry.sweep_at = held.saturating_add(SWEEP_MARGIN);
        }
    }

    /// Records `fd`, just received from `client` as `kind`, standing for
    /// `weight` fds (at least 1). Any older record on the same number is dead
    /// (the number could not have been reused otherwise) and is dropped: the
    /// insert that records this one hands back the old owner, so this costs
    /// no lookup of its own.
    pub(crate) fn record(
        &mut self,
        client: &ClientId,
        fd: RawFd,
        kind: Kind,
        check: Check,
        weight: u8,
    ) {
        if let Some(previous) = self.owner.insert(fd, client.clone()) {
            self.drop_record(&previous, fd);
        }
        let weight = weight.max(1);
        let spare = &mut self.spare;
        let held = self
            .per_client
            .entry(client.clone())
            .or_insert_with(|| Held {
                records: spare.pop().unwrap_or_default(),
                ..Held::default()
            });
        held.records.push(Record {
            fd,
            kind,
            check,
            weight,
        });
        held.weight = held.weight.saturating_add(u32::from(weight));
        if kind == Kind::Timeline {
            held.timelines += 1;
        }
    }

    /// Lowers the record on `fd` to 1 plus `copies`, if it weighs more: the
    /// plane on that number has just been imported, and the session now knows
    /// the renderer keeps `copies` fds for it (see `dmabuf/renderer_copies.rs`).
    /// Only ever lowers. A plane is admitted at the weight the session
    /// expects its copies to cost, so raising it here would take the client
    /// past a bound it was admitted under; see that module for when the
    /// expectation can fall short. A number with no record is one failed
    /// lookup.
    pub(crate) fn settle_copies(&mut self, fd: RawFd, copies: u8) {
        let Some(client) = self.owner.get(&fd) else {
            return;
        };
        let Some(held) = self.per_client.get_mut(client) else {
            return;
        };
        if let Some(record) = held.records.iter_mut().find(|record| record.fd == fd) {
            let settled = copies.saturating_add(1);
            if settled < record.weight {
                held.weight = held
                    .weight
                    .saturating_sub(u32::from(record.weight - settled));
                record.weight = settled;
            }
        }
    }

    /// Whether any client's pool record names the file with this identity:
    /// a pool is open on it, or was until too recently for the fd to have
    /// closed. For the renderer probe in `dmabuf/renderer_copies.rs`, which
    /// must not measure while a pool's fd on the same file can close under
    /// it. Walks every record; it runs only until the session's first clean
    /// measurement.
    pub(crate) fn pool_names(&self, dev: u64, ino: u64) -> bool {
        self.per_client
            .values()
            .flat_map(|held| &held.records)
            .any(|record| record.kind == Kind::Pool && record.check == Check::Same { dev, ino })
    }

    /// `fd` has just been received from a client, so whatever this ledger
    /// recorded on that number was closed in between: forget it. One map
    /// lookup when nothing is recorded on it, which is the usual case.
    pub(crate) fn fd_arrived(&mut self, fd: RawFd) {
        if self.owner.is_empty() {
            return;
        }
        if let Some(client) = self.owner.remove(&fd) {
            self.drop_record(&client, fd);
        }
    }

    /// Drops `client`'s record on `fd`, whose owner entry is already gone,
    /// and releases the client's entry if that was its last record.
    fn drop_record(&mut self, client: &ClientId, fd: RawFd) {
        let Some(held) = self.per_client.get_mut(client) else {
            return;
        };
        if let Some(at) = held.records.iter().position(|record| record.fd == fd) {
            let record = held.records.swap_remove(at);
            held.forget(&record);
        }
        if held.records.is_empty() {
            self.release_entry(client);
        }
    }

    /// Removes `client`'s (empty) entry, keeping its record list for the
    /// next entry that needs one, so a client whose records keep emptying --
    /// one that allocates and frees a pool at a time -- does not allocate a
    /// list per arrival. At most [`SPARE_LISTS`] are kept, each shrunk to
    /// [`SPARE_LIST_CAPACITY`], so the spares cost a few KiB at most.
    fn release_entry(&mut self, client: &ClientId) {
        let Some(held) = self.per_client.remove(client) else {
            return;
        };
        if self.spare.len() < SPARE_LISTS {
            let mut records = held.records;
            records.clear();
            records.shrink_to(SPARE_LIST_CAPACITY);
            self.spare.push(records);
        }
    }

    /// Checks each of `client`'s records with `still_held`, drops the dead
    /// ones, and answers what is left: the weighted total, every kind, and
    /// the timelines alone. Sets the pressure amortization mark to the total
    /// plus [`SWEEP_MARGIN`].
    ///
    /// `still_held` is `liveness::still_held` in production. It is a
    /// parameter so that the bookkeeping is unit-testable without real fds.
    pub(crate) fn sweep(
        &mut self,
        client: &ClientId,
        mut still_held: impl FnMut(RawFd, Check) -> bool,
    ) -> (u32, u32) {
        let Some(held) = self.per_client.get_mut(client) else {
            return (0, 0);
        };
        let owner = &mut self.owner;
        let (mut timelines, mut weight) = (0u32, 0u32);
        held.records.retain(|record| {
            let live = still_held(record.fd, record.check);
            if live {
                timelines += u32::from(record.kind == Kind::Timeline);
                weight = weight.saturating_add(u32::from(record.weight));
            } else if owner.get(&record.fd) == Some(client) {
                owner.remove(&record.fd);
            }
            live
        });
        held.timelines = timelines;
        held.weight = weight;
        held.sweep_at = weight.saturating_add(SWEEP_MARGIN);
        if held.records.is_empty() {
            self.release_entry(client);
        }
        (weight, timelines)
    }

    /// How many records the ledger holds for every client. Test-only.
    #[cfg(test)]
    pub(crate) fn records(&self) -> usize {
        debug_assert_eq!(
            self.owner.len(),
            self.per_client
                .values()
                .map(|held| held.records.len())
                .sum::<usize>(),
            "every record is in both maps"
        );
        debug_assert!(
            self.per_client.values().all(|held| held.timelines as usize
                == held
                    .records
                    .iter()
                    .filter(|record| record.kind == Kind::Timeline)
                    .count()),
            "every client's timeline count matches its records"
        );
        debug_assert!(
            self.per_client.values().all(|held| held.weight
                == held
                    .records
                    .iter()
                    .map(|record| u32::from(record.weight))
                    .sum::<u32>()),
            "every client's weight matches its records"
        );
        self.owner.len()
    }

    /// Sweeps every client with the real checks and answers how many fds of
    /// `kind` (every kind, for `None`) this process still keeps for all of
    /// them. Test-only.
    #[cfg(test)]
    pub(crate) fn in_flight(&mut self, kind: Option<Kind>) -> u32 {
        let clients: Vec<ClientId> = self.per_client.keys().cloned().collect();
        for client in &clients {
            self.sweep(client, liveness::still_held);
        }
        self.per_client
            .values()
            .flat_map(|held| &held.records)
            .filter(|record| kind.is_none_or(|kind| record.kind == kind))
            .count() as u32
    }

    /// The file identities of `client`'s pool and plane records. Test-only:
    /// lets a wire test count the fds this process holds on exactly those
    /// files (renderer copies included, which share them), and no other
    /// test's.
    #[cfg(test)]
    pub(crate) fn identities_of(&self, client: &ClientId) -> Vec<(u64, u64)> {
        self.per_client
            .get(client)
            .into_iter()
            .flat_map(|held| &held.records)
            .filter_map(|record| match record.check {
                Check::Same { dev, ino } => Some((dev, ino)),
                Check::Syncobj | Check::Open => None,
            })
            .collect()
    }

    /// [`Self::in_flight`] for timelines alone, as `drm_syncobj`'s tests read
    /// it. Test-only.
    #[cfg(test)]
    pub(crate) fn timelines_in_flight(&mut self) -> u32 {
        self.in_flight(Some(Kind::Timeline))
    }
}
