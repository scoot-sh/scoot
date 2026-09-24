//! Which imported syncobj timeline fds this process still holds, and for
//! which client: the ledger behind [`MAX_TIMELINES_PER_CLIENT`].
//!
//! ## What has to be counted, and why an object count cannot
//!
//! `import_timeline` hands scoot a syncobj fd, and Smithay keeps it in the
//! timeline's `DrmTimelineInner` (as `timeline_fd`) until that inner value is
//! dropped. It is dropped when the last `DrmTimeline` clone goes, not when the
//! `wp_linux_drm_syncobj_timeline_v1` object is destroyed, because every
//! `DrmSyncPoint` on the timeline holds a clone. The protocol requires points
//! to outlive their timeline object ("destroying a timeline does not unset
//! points"), and they live where scoot cannot count them:
//!
//! - a surface's pending points, set and never committed;
//! - its committed points;
//! - every commit queued behind one still waiting on its acquire point
//!   (Smithay's per-surface cache `VecDeque`), each with its own pair;
//! - the renderer's `Buffer` for the committed buffer, and the clones of it
//!   that the release hold and the scanout keep for frames in flight.
//!
//! So a count released on object destroy, which is what scoot had, lets a
//! client import, set points on a fresh surface and destroy the timeline, and
//! keep one fd here per surface with none counted. Review of PR #233 measured
//! 440 such surfaces holding 927 fds with zero live timelines. New clients
//! were shed, and the offender was not killed.
//!
//! Scoot cannot watch the `DrmTimeline`s themselves. `DrmTimeline` wraps a
//! `pub(super)` `Arc`, so no scoot code can hold a `Weak` to one or read its
//! count. And scoot stores no points of its own: every retention site above
//! is Smithay's. The Smithay fork is approved for the handle-leak `Drop` only
//! (`docs/backlog/resolved/syncobj-handle-leak-done.md`), so a hook there is
//! off the table.
//!
//! ## The fd is the timeline
//!
//! `DrmTimelineInner` owns exactly one fd, and it is the very `OwnedFd` from
//! the request, moved unchanged (`DrmTimeline::new` at the pinned rev). The
//! syncobj *handle* it imports is not an fd. So "the timeline was freed" and
//! "that fd number was closed" are the same event, and scoot can observe the
//! second. The ledger records the fd number of every import against its
//! client, before delegation. It learns that a record is dead in two ways,
//! both exact:
//!
//! - **The number arrives again.** The kernel reuses only closed numbers, so
//!   any fd number reaching scoot through `import_timeline`, `add` or
//!   `create_pool` proves that the record on that number is dead
//!   ([`RetainedTimelines::fd_arrived`]).
//! - **A sweep checks it.** `fcntl(F_GETFD)` says whether the number is
//!   open, and `readlink(/proc/self/fd/N)` says whether it is still a
//!   syncobj (`anon_inode:syncobj_file` on the dev VM's 6.18 kernel). A
//!   number reused by anything else (an eventfd, a socket, a dma-buf) reads
//!   as something else.
//!
//! A number that is closed and reopened *as another syncobj file*, by a path
//! that does not pass through the invalidation above, would read as live.
//! This process opens no syncobj fds of its own: its syncobjs are handles on
//! the import device, and the sync files it exports read as
//! `anon_inode:sync_file`. So the only way is a client fd arriving through
//! some other request. Of the fd-carrying requests scoot serves, the
//! retaining ones are all invalidated above. The rest (`receive` on the
//! selection offers, `set_gamma`) hold their fd only until the same
//! dispatch's flush or close. There is one more place: wayland-backend
//! buffers a client's received fds until their message is parsed. A client
//! can use either to keep an old record reading live for as long as that
//! fd lives. That over-counts the record's owner and never under-counts
//! anyone, so it cannot open a hole in the bound. It is also far less than
//! the second path already allows: see
//! `docs/backlog/core/wayland-backend-unbounded-incoming-fds.md`.
//!
//! ## When the bound is checked
//!
//! Only at `import_timeline`, the one request that adds a record. The
//! records ([`RetainedTimelines::held_by`]) are an upper bound on what the
//! client really holds: every retained fd has one, and dead ones linger until
//! something notices. Refusing on that number alone would kill a legitimate
//! client whose swapchain churn left dead records. So a refusal is only ever
//! decided on a fresh sweep. See `super::reject_excess_timeline` for the
//! rule, and [`SWEEP_MARGIN`] for how the sweeps are amortized.
//!
//! ## Bounds on the ledger itself
//!
//! Each fd number has at most one record, since a new record on a number
//! replaces the old one. So the ledger never holds more records than the fd
//! table has numbers, however many clients come and go. A disconnected
//! client's records are never swept (only an import sweeps, and only its
//! own records), so they stay until their numbers are reused. That is
//! bounded the same way, so no disconnect hook is needed.

use std::collections::HashMap;
use std::mem::MaybeUninit;
use std::os::fd::{BorrowedFd, RawFd};

use smithay::reexports::wayland_server::backend::ClientId;

#[cfg(doc)]
use super::MAX_TIMELINES_PER_CLIENT;

#[cfg(test)]
mod tests;

/// How far below its bound a sweep must bring a client for the import that
/// triggered it to be admitted. After such a sweep the client can import
/// this many more before the next one.
///
/// This is what keeps sweeps off the per-request path. Without it, a client
/// that really holds one fewer than the cap and churns (import, destroy,
/// import) would trigger a sweep, which is ~128 `fcntl` + `readlink` pairs,
/// on every import, paced by the attacker. With it, sweeps are at most one
/// per 16 imports. For the hard cap, a client whose sweep leaves more than
/// `MAX - 16` = 112 live timelines is refused, which is 7x the 16 a Vulkan
/// window uses. Under fd pressure the margin is admission slack instead: a
/// client swept to exactly the grace may import up to 16 more before it is
/// swept again. The grace is where a legitimate client sits, so it must not
/// shrink.
pub(crate) const SWEEP_MARGIN: u32 = 16;

/// Why [`RetainedTimelines::admit`] refused an import, with how many
/// timelines the fresh sweep found the client still holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// At the hard cap, and a sweep could not free [`SWEEP_MARGIN`].
    Cap { held: u32 },
    /// Past the pressure grace while the fd table is pressured.
    Pressure { held: u32 },
}

/// The ledger. See the module doc.
#[derive(Debug, Default)]
pub(crate) struct RetainedTimelines {
    /// Which client imported the timeline whose fd holds each number. At most
    /// one record per number, so this is bounded by the fd table's size.
    owner: HashMap<RawFd, ClientId>,
    /// Each client's records, as a list for its own sweep.
    per_client: HashMap<ClientId, Held>,
}

/// One client's records.
#[derive(Debug, Default)]
struct Held {
    /// The fd numbers recorded for this client. Unordered, and never longer
    /// than `MAX_TIMELINES_PER_CLIENT`: an import that would take it past
    /// that either sweeps it back down first or is refused.
    fds: Vec<RawFd>,
    /// The pressure-grace sweep amortization: no pressure sweep until the
    /// records reach this. Set to live + [`SWEEP_MARGIN`] by every sweep, and
    /// 0 (meaning due) before the first.
    sweep_at: u32,
}

impl RetainedTimelines {
    /// How many timeline fds are recorded for `client`. An upper bound on
    /// how many it really has this process hold (see the module doc).
    pub(crate) fn held_by(&self, client: &ClientId) -> u32 {
        self.per_client
            .get(client)
            .map_or(0, |held| held.fds.len() as u32)
    }

    /// Whether a pressure sweep of `client` is due: its records have grown
    /// [`SWEEP_MARGIN`] past what its last sweep left, or it was never swept.
    pub(crate) fn sweep_due(&self, client: &ClientId) -> bool {
        self.per_client
            .get(client)
            .is_some_and(|held| held.fds.len() as u32 >= held.sweep_at)
    }

    /// Decides whether `client` may import one more timeline, against a
    /// hard `cap` and a `grace` that applies only while `pressured()` says
    /// the fd table is. It does not record anything; the caller records the
    /// fd once the import is admitted.
    ///
    /// Both refusals are decided on a fresh sweep, never on the raw record
    /// count, which may include dead records:
    ///
    /// - **Cap.** The records never exceed `cap`, because an import that
    ///   finds them at `cap` sweeps first. It is refused if the sweep leaves
    ///   more than `cap - SWEEP_MARGIN` live. Otherwise the client has at
    ///   least [`SWEEP_MARGIN`] imports before it can reach `cap` again,
    ///   which is what amortizes the sweeps.
    /// - **Pressure grace.** Past `grace` records, and only if a sweep is due
    ///   ([`Self::sweep_due`]) or has just run for the cap, the table is
    ///   observed (`pressured`, a `/proc/self/fd` readdir in production). If it
    ///   is pressured, the client is refused if a sweep leaves it past
    ///   `grace`. A sweep that does not refuse sets the next one
    ///   [`SWEEP_MARGIN`] records later, so under pressure a client can hold
    ///   up to `grace + SWEEP_MARGIN`.
    ///
    /// `pressured` is evaluated at most once, and only past the grace, which
    /// no legitimate client reaches (see `PRESSURE_GRACE_TIMELINES`). So the
    /// import path of every well-behaved client costs a map lookup here, and
    /// no syscall.
    pub(crate) fn admit(
        &mut self,
        client: &ClientId,
        cap: u32,
        grace: u32,
        mut still_held: impl FnMut(RawFd) -> bool,
        pressured: impl FnOnce() -> bool,
    ) -> Result<(), Refusal> {
        let mut held = self.held_by(client);
        let mut fresh = false;
        if held >= cap {
            held = self.sweep(client, &mut still_held);
            fresh = true;
            if held > cap.saturating_sub(SWEEP_MARGIN) {
                return Err(Refusal::Cap { held });
            }
        }
        if held > grace && (fresh || self.sweep_due(client)) && pressured() {
            if !fresh {
                held = self.sweep(client, &mut still_held);
            }
            if held > grace {
                return Err(Refusal::Pressure { held });
            }
        }
        Ok(())
    }

    /// Records `fd`, just received in `client`'s `import_timeline`. Any older
    /// record on the same number is dead (the number could not have been
    /// reused otherwise), and is dropped first.
    pub(crate) fn record(&mut self, client: &ClientId, fd: RawFd) {
        self.fd_arrived(fd);
        self.owner.insert(fd, client.clone());
        self.per_client
            .entry(client.clone())
            .or_default()
            .fds
            .push(fd);
    }

    /// `fd` has just been received from a client, so whatever this ledger
    /// recorded on that number was closed in between: forget it. One
    /// `is_empty` test while nothing is recorded, which is every session
    /// that does not offer explicit sync.
    pub(crate) fn fd_arrived(&mut self, fd: RawFd) {
        if self.owner.is_empty() {
            return;
        }
        let Some(client) = self.owner.remove(&fd) else {
            return;
        };
        if let Some(held) = self.per_client.get_mut(&client) {
            if let Some(at) = held.fds.iter().position(|&recorded| recorded == fd) {
                held.fds.swap_remove(at);
            }
            if held.fds.is_empty() {
                self.per_client.remove(&client);
            }
        }
    }

    /// Checks each of `client`'s records with `still_held`, drops the dead
    /// ones, and answers how many are left. Sets the pressure amortization
    /// mark to that plus [`SWEEP_MARGIN`].
    ///
    /// `still_held` is [`timeline_fd_open`] in production. It is a parameter
    /// so that the bookkeeping is unit-testable without real syncobjs.
    pub(crate) fn sweep(&mut self, client: &ClientId, mut still_held: impl FnMut(RawFd) -> bool) -> u32 {
        let Some(held) = self.per_client.get_mut(client) else {
            return 0;
        };
        let owner = &mut self.owner;
        held.fds.retain(|&fd| {
            let live = still_held(fd);
            if !live && owner.get(&fd) == Some(client) {
                owner.remove(&fd);
            }
            live
        });
        let live = held.fds.len() as u32;
        held.sweep_at = live.saturating_add(SWEEP_MARGIN);
        if live == 0 {
            self.per_client.remove(client);
        }
        live
    }

    /// How many records the ledger holds for every client. Test-only.
    #[cfg(test)]
    pub(crate) fn records(&self) -> usize {
        debug_assert_eq!(
            self.owner.len(),
            self.per_client.values().map(|held| held.fds.len()).sum::<usize>(),
            "every record is in both maps"
        );
        self.owner.len()
    }

    /// Sweeps every client with the real check and answers how many
    /// timelines this process still holds between them. Test-only.
    #[cfg(test)]
    pub(crate) fn sweep_all(&mut self) -> u32 {
        let clients: Vec<ClientId> = self.per_client.keys().cloned().collect();
        clients
            .iter()
            .map(|client| self.sweep(client, timeline_fd_open))
            .sum()
    }
}

/// What `readlink(/proc/self/fd/N)` reads for a DRM syncobj fd. It is the
/// name `drm_syncobj.c` gives `anon_inode_getfile`. Verified on the dev VM
/// (kernel 6.18): an exported timeline syncobj reads exactly this, an eventfd
/// reads `anon_inode:[eventfd]`, and a memfd reads `/memfd:<name> (deleted)`.
const SYNCOBJ_LINK: &[u8] = b"anon_inode:syncobj_file";

/// Whether fd number `fd` is still open in this process and is still a
/// syncobj, and so presumably the timeline fd recorded on it.
///
/// Two syscalls and no allocation (both paths and the link are in stack
/// buffers). `F_GETFD` answers "open?" definitively, and does not need
/// `/proc`. The type check does need it, so on a system without `/proc` an
/// open fd counts as held. That is conservative: it can only over-count, and
/// only the fd's own recorded client.
pub(crate) fn timeline_fd_open(fd: RawFd) -> bool {
    if fd < 0 {
        return false;
    }
    // SAFETY: the borrow lives only for this `fcntl`, which never closes or
    // otherwise changes the fd, and a number that is not open simply fails
    // with `EBADF`. Nothing is read or written through it.
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    if rustix::io::fcntl_getfd(borrowed).is_err() {
        return false;
    }
    let mut path = [0u8; 32];
    let Some(path) = proc_fd_path(fd, &mut path) else {
        return true;
    };
    let mut link = [MaybeUninit::<u8>::uninit(); 64];
    match rustix::fs::readlinkat_raw(rustix::fs::CWD, path, &mut link) {
        Ok((read, _)) => &*read == SYNCOBJ_LINK,
        Err(_) => true,
    }
}

/// `/proc/self/fd/<fd>` as a C string in `buf`, or `None` if it does not fit
/// (it always does: 14 bytes of prefix, at most 10 digits and the nul).
fn proc_fd_path(fd: RawFd, buf: &mut [u8; 32]) -> Option<&std::ffi::CStr> {
    const PREFIX: &[u8] = b"/proc/self/fd/";
    buf[..PREFIX.len()].copy_from_slice(PREFIX);
    let mut digits = [0u8; 10];
    let mut value = u32::try_from(fd).ok()?;
    let mut count = 0;
    loop {
        digits[count] = b'0' + (value % 10) as u8;
        count += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    let end = PREFIX.len() + count;
    for (slot, digit) in buf[PREFIX.len()..end]
        .iter_mut()
        .zip(digits[..count].iter().rev())
    {
        *slot = *digit;
    }
    buf[end] = 0;
    std::ffi::CStr::from_bytes_with_nul(&buf[..=end]).ok()
}
