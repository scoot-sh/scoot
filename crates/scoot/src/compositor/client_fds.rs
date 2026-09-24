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
//! checked). That duplicate is not the client's fd, so no arrival
//! records it; `dmabuf/renderer_copies.rs` learns once per session how many
//! the renderer keeps and adds them to the plane's record as it is imported
//! ([`ClientFds::add_copies`]). So a record has a *weight* -- 1, plus its
//! copies -- and every bound here reads the weighted sum: the fds a client
//! really makes this process hold. A copy can outlive its plane until the
//! renderer's cache is next drained; that module states the window.
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
//! 512 here (renderer copies included), plus 64 acquire-wait eventfds
//! (`drm_syncobj/acquire.rs`, scoot's own fds, bounded there), plus its
//! socket: **577**. Against the idle baseline measured there (43) that is
//! 620, 276 below the pressure line of a 1024-fd table (896); measured live
//! at the bound, 557. So one connection cannot trip the reserve on its own
//! through anything scoot counts, on any tier. On a software GLES renderer
//! add the copies of planes that closed since the renderer's cache was last
//! drained (`dmabuf/renderer_copies.rs`): at most the one round of the
//! client's buffers, which at two fds a plane is 256 more, 876 in all --
//! still under. What scoot does not count is in `fd_pressure.rs`.
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
/// An arrival that finds the client at this bound sweeps its records first,
/// and is refused only if more than `512 - SWEEP_MARGIN` = 496 of them are
/// still open. So a client never has more than 512 fds kept here. The refusal
/// is the arriving request's own (see [`Refusal`]), and kills only that
/// client.
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
}

/// One recorded fd.
#[derive(Debug, Clone, Copy)]
struct Record {
    fd: RawFd,
    kind: Kind,
    /// How a sweep tells whether the fd on this number is still this one.
    check: Check,
    /// How many fds this record stands for: 1, plus the copies a renderer
    /// keeps of an imported plane's fd ([`ClientFds::add_copies`]). Always at
    /// least 1.
    weight: u8,
}

/// One client's records.
#[derive(Debug, Default)]
struct Held {
    /// Unordered, and never longer than [`Limits::total`]: every record
    /// weighs at least 1, and an arrival that finds the weight at that bound
    /// either sweeps it back down first or is refused. (The weight itself can
    /// pass the bound by the copies of the one import after the last
    /// admitted `add`; the next arrival then finds it there.)
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
    ) -> Result<(), Refusal> {
        self.fd_arrived(fd);
        self.admit(client, kind, LIMITS, liveness::still_held, || {
            super::fd_pressure::table().is_some_and(|table| table.pressured())
        })
    }

    /// Records `fd`, just admitted from `client` as `kind`, with the identity
    /// check a later sweep will use. One `fstat` for a pool or a plane, none
    /// for a timeline, plus one insert in each of two maps.
    pub(crate) fn record_arrival(&mut self, client: &ClientId, fd: BorrowedFd<'_>, kind: Kind) {
        let check = liveness::capture(fd, kind);
        self.record(client, fd.as_raw_fd(), kind, check);
    }

    /// Decides whether `client` may have scoot keep one more fd of `kind`,
    /// against `limits`, with the grace applying only while `pressured()`
    /// says the fd table is. Records nothing.
    ///
    /// Every refusal is decided on a fresh sweep, never on the raw record
    /// count, which may include dead records:
    ///
    /// - **Caps.** The records never exceed `limits.total`, because an
    ///   arrival that finds them there sweeps first. It is refused if the
    ///   sweep leaves more than `limits.total - SWEEP_MARGIN` live. The same
    ///   for a timeline import against `limits.timelines`, counting timelines
    ///   only. Otherwise the client has at least [`SWEEP_MARGIN`] arrivals
    ///   before it can reach the bound again, which is what amortizes the
    ///   sweeps.
    /// - **Pressure grace.** Past `limits.grace` records, and only if a check
    ///   is due ([`Self::sweep_due`]) or a sweep has just run for a cap, the
    ///   table is observed (`pressured`, a `/proc/self/fd` readdir in
    ///   production). If it is pressured, the client is refused if a sweep
    ///   leaves it past the grace. Either way the next check is
    ///   [`SWEEP_MARGIN`] records later: a sweep that does not refuse sets it
    ///   from what the sweep left, and a calm observation from the records as
    ///   they stand. So at most `SWEEP_MARGIN` arrivals pass between checks.
    ///
    /// `pressured` is evaluated at most once per call, and at most once per
    /// [`SWEEP_MARGIN`] arrivals per client. Under the grace, which is where
    /// every well-behaved client is, this is two map lookups and no syscall.
    pub(crate) fn admit(
        &mut self,
        client: &ClientId,
        kind: Kind,
        limits: Limits,
        mut still_held: impl FnMut(RawFd, Check) -> bool,
        pressured: impl FnOnce() -> bool,
    ) -> Result<(), Refusal> {
        let mut total = self.held_by(client);
        let at_timeline_cap =
            kind == Kind::Timeline && self.timelines_held_by(client) >= limits.timelines;
        let mut fresh = false;
        if total >= limits.total || at_timeline_cap {
            let (live, timelines) = self.sweep(client, &mut still_held);
            total = live;
            fresh = true;
            if total > limits.total.saturating_sub(SWEEP_MARGIN) {
                return Err(Refusal::Total { held: total });
            }
            if kind == Kind::Timeline && timelines > limits.timelines.saturating_sub(SWEEP_MARGIN) {
                return Err(Refusal::Timelines { held: timelines });
            }
        }
        if total > limits.grace && (fresh || self.sweep_due(client)) {
            if !pressured() {
                // Calm: not again until another margin's worth of arrivals.
                self.check_again_after(client, total);
                return Ok(());
            }
            if !fresh {
                total = self.sweep(client, &mut still_held).0;
            }
            if total > limits.grace {
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

    /// Records `fd`, just received from `client` as `kind`. Any older record
    /// on the same number is dead (the number could not have been reused
    /// otherwise), and is dropped first.
    pub(crate) fn record(&mut self, client: &ClientId, fd: RawFd, kind: Kind, check: Check) {
        self.fd_arrived(fd);
        self.owner.insert(fd, client.clone());
        let held = self.per_client.entry(client.clone()).or_default();
        held.records.push(Record {
            fd,
            kind,
            check,
            weight: 1,
        });
        held.weight = held.weight.saturating_add(1);
        if kind == Kind::Timeline {
            held.timelines += 1;
        }
    }

    /// Adds `copies` to the weight of the record on `fd`: a renderer has just
    /// imported the plane on that number and keeps that many fds of its own
    /// for it (see `dmabuf.rs`'s `renderer_plane_copies`). A number with no
    /// record, which only a plane recorded before a disconnect could be, is
    /// one failed lookup. Saturating: a record never weighs more than
    /// `u8::MAX`, far past any renderer count.
    pub(crate) fn add_copies(&mut self, fd: RawFd, copies: u8) {
        let Some(client) = self.owner.get(&fd) else {
            return;
        };
        let Some(held) = self.per_client.get_mut(client) else {
            return;
        };
        if let Some(record) = held.records.iter_mut().find(|record| record.fd == fd) {
            let before = record.weight;
            record.weight = record.weight.saturating_add(copies);
            held.weight = held
                .weight
                .saturating_add(u32::from(record.weight - before));
        }
    }

    /// `fd` has just been received from a client, so whatever this ledger
    /// recorded on that number was closed in between: forget it. One map
    /// lookup when nothing is recorded on it, which is the usual case.
    pub(crate) fn fd_arrived(&mut self, fd: RawFd) {
        if self.owner.is_empty() {
            return;
        }
        let Some(client) = self.owner.remove(&fd) else {
            return;
        };
        let Some(held) = self.per_client.get_mut(&client) else {
            return;
        };
        if let Some(at) = held.records.iter().position(|record| record.fd == fd) {
            let record = held.records.swap_remove(at);
            held.forget(&record);
        }
        if held.records.is_empty() {
            self.per_client.remove(&client);
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
            self.per_client.remove(client);
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
