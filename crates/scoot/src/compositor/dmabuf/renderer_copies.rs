//! The fds a GLES renderer keeps of its own for each dma-buf plane it
//! imports, charged to the client that sent the plane.
//!
//! The fd ledger (`client_fds.rs`) records every plane's fd as it arrives.
//! That is every fd a *client* hands over, but not every fd its buffer makes
//! this process hold: Mesa's software rasterizer (llvmpipe, which is what
//! the dev VM's `--renderer gles` and `--tty` GPU tier run on, through
//! `kms_swrast`) duplicates each imported plane's fd for its own mapping,
//! and keeps the duplicate for as long as the renderer's texture cache holds
//! the import. Measured on the dev VM: a three-plane `YU12` buffer imported
//! and committed takes six dma-buf fds and pixman's one udmabuf plane takes
//! one (both pinned in `client_fds/tests/dmabuf.rs`), and a single-plane
//! `XR24` buffer two (a one-off diagnostic in the change that added this).
//! Before this was counted, that change's fail-first run measured 200 such
//! `YU12` buffers holding 1200 dma-buf fds on `main`: 600 planes, and 600
//! copies nobody counted. A hardware driver imports a dma-buf into a
//! GEM handle and keeps no fd (reasoned from Mesa's gallium winsys code, not
//! measured: no hardware GPU is reachable from here; `Asahi.md` has the
//! check).
//!
//! So the copies are learned rather than assumed. On the session's first
//! cleanly measurable import into a GLES backend, [`Probe`] counts the fds
//! in this process that name the buffer's first plane's file (by `fstat`
//! identity) just before the import and again just after, and divides the
//! difference by the planes on that file and the GLES backends it went
//! into. An import that cannot be measured cleanly (see [`Probe::before`]
//! and [`learn`]) is charged one copy per plane per backend and teaches
//! nothing; the next one is measured. The
//! difference, not the count: anything that already named the file -- the
//! planes themselves, or fds a client parked in wayland-backend's received-fd
//! queue on its own buffer (`docs/backlog/core/wayland-backend-fd-queue.md`)
//! -- is in both counts and cancels, so no client can teach the session a
//! bigger number. Nothing else opens an fd on a client's dma-buf in between;
//! the only other thread touching fds (Smithay's shm drop thread) only closes
//! pool fds. That ratio -- 1 on llvmpipe, 0 on a driver that keeps nothing --
//! is kept for the session (its GLES device is pinned; see
//! `render/gles.rs`).
//!
//! **Charged at admission, not at import.** A plane is admitted into the fd
//! ledger at its full weight, [`plane_weight`]: 1, plus that ratio for each
//! GLES backend, or plus 1 per backend while the session has not learned
//! the ratio yet. An import then only lowers a plane's weight to what the
//! session has learned ([`charge`]). The copies used to be added at import,
//! after the bound had been checked, and review of PR #239 took a client to
//! 540 fds against its 512 that way (planes added near the bound, then
//! imported); `client_fds/tests/dmabuf.rs` pins the shape. One case can
//! still fall short, and only on a driver that keeps more than one copy per
//! plane, which none measured does: planes admitted before the session's
//! first clean measurement were charged one copy per backend, and are not
//! raised afterwards. At most the planes in flight until then, each short by
//! the difference.
//!
//! The measurement costs two `/proc/self/fd` walks with an `fstat` per
//! entry, once per session (again only after an import that could not be
//! measured); after that, a map lookup per plane per import (imports happen
//! when a client allocates a buffer, not per frame).
//!
//! **What this does not count.** A copy lives until the renderer's cache
//! drops the import, which is after the plane's `Dmabuf` is gone *and* a
//! cache drain or a frame has run. The ledger forgets the record when the
//! plane closes, so in between the copy is held and not counted. The drain is
//! scheduled on every `wl_buffer` and `wl_surface` destruction
//! (`schedule_cache_drain`), which covers the ways a client drops a buffer
//! but one: a commit that replaces a buffer whose `wl_buffer` was already
//! destroyed, on a surface nothing redraws, leaves its copies until the next
//! drain. Any later `wl_buffer` destruction by any client drains them, so
//! they do not pile up past one round of a client's buffers: at most 256
//! copies at two fds a plane, on top of the 512. In practice the window is
//! short: the drain runs at the loop's next idle, and review of PR #239 saw
//! all 240 copies of 80 released three-plane buffers close within 500 ms
//! on both GLES tiers. An output added after a plane was admitted -- before
//! its import, or after it, when the new output makes its own copy on its
//! first frame of that buffer -- is uncounted for that plane.

use std::os::fd::{AsRawFd, BorrowedFd};

use smithay::backend::allocator::dmabuf::Dmabuf;

use crate::cli::RendererKind;
use crate::compositor::State;

#[cfg(test)]
mod tests;

/// The most copies per plane per backend a probe will believe: a backstop,
/// since the before/after difference already leaves out whatever else names
/// the buffer. Past it a count is more likely something unforeseen than a
/// renderer, and charging it would shrink every client's budget for the rest
/// of the session.
const MAX_COPIES_PER_PLANE: u8 = 4;

/// The first half of the once-per-session measurement: the fds naming the
/// buffer's first plane's file, counted before the import. See the module
/// doc.
pub(in crate::compositor) struct Probe {
    dev: u64,
    ino: u64,
    before: usize,
}

impl Probe {
    /// Counts, if this import can be learned from: the session has not
    /// learned its renderer's copies yet, has a GLES backend, and no shm pool
    /// is open on the buffer's file. `None` otherwise, and where the count
    /// cannot be taken (no identity for the plane, no `/proc`); [`charge`]
    /// then charges that one import as if the renderer kept a copy, and
    /// learns from a later one.
    ///
    /// Why a pool on the same file disqualifies it: a `wl_shm` pool may be
    /// backed by any mappable fd, a dma-buf included, and a dropped pool's fd
    /// is closed on Smithay's own drop thread, concurrently with this
    /// thread's two counts. A client could have that thread close fds on its
    /// buffer's file in between, and push the difference down -- toward
    /// teaching the session that the renderer keeps nothing, the direction
    /// that lets the table fill. Every such fd has a pool record in the fd
    /// ledger until it has really closed, so the ledger says when that is
    /// possible. (Reasoned from the drop thread's code; not reproduced.)
    pub(in crate::compositor) fn before(state: &State, dmabuf: &Dmabuf) -> Option<Self> {
        if state.renderer_plane_copies.is_some() || gles_backends(state) == 0 {
            return None;
        }
        let (dev, ino) = identity(dmabuf.handles().next()?)?;
        if state.client_fds.pool_names(dev, ino) {
            return None;
        }
        let before = fds_naming(dev, ino)?;
        Some(Self { dev, ino, before })
    }

    /// What this import teaches: the copies per plane per backend it made,
    /// or `None` if the second count cannot be taken or the measurement was
    /// disturbed (see [`learn`]).
    fn copies(&self, dmabuf: &Dmabuf, backends: usize) -> Option<u8> {
        let after = fds_naming(self.dev, self.ino)?;
        let planes = dmabuf
            .handles()
            .filter(|plane| identity(*plane) == Some((self.dev, self.ino)))
            .count();
        learn(self.before, after, planes, backends)
    }
}

/// The copies per plane per backend from counts `before` and `after` an
/// import of `planes` planes (on the probed file) into `backends` GLES
/// backends. `None` when `after < before`: something closed fds on the file
/// while the import ran, so the difference is not the renderer's, and a low
/// reading is the dangerous one to keep. Split out to pin the arithmetic
/// without a renderer.
fn learn(before: usize, after: usize, planes: usize, backends: usize) -> Option<u8> {
    let copies = after.checked_sub(before)?;
    Some(per_plane(copies, planes, backends))
}

/// How many backends a dma-buf import goes into as GLES.
fn gles_backends(state: &State) -> usize {
    state
        .backends
        .values()
        .filter(|backend| backend.renderer() == RendererKind::Gles)
        .count()
}

/// The weight a plane arriving now is admitted at in the fd ledger: 1, plus
/// the copies each GLES backend will make of it when it is imported -- the
/// session's learned number, or 1 until it has learned one (the
/// conservative direction; see the module doc). 1 on a session with no
/// GLES backend. Charging the copies here, at admission, is what keeps an
/// import from taking a client past a bound it was already admitted under.
pub(in crate::compositor) fn plane_weight(state: &State) -> u8 {
    let backends = gles_backends(state);
    if backends == 0 {
        return 1;
    }
    let per_backend = state.renderer_plane_copies.unwrap_or(1);
    per_backend
        .saturating_mul(u8::try_from(backends).unwrap_or(u8::MAX))
        .saturating_add(1)
}

/// Settles `dmabuf`'s planes, just imported into every backend: learns the
/// renderer's copies from `probe` if the session has not yet (see the module
/// doc), and lowers the planes' ledger weights to what they really cost
/// once it knows. The weights were charged at admission
/// ([`plane_weight`]), so this never raises one. Until a clean measurement
/// the planes keep the one copy per backend they were admitted at. Nothing
/// at all on a session with no GLES backend.
pub(in crate::compositor) fn charge(state: &mut State, dmabuf: &Dmabuf, probe: Option<Probe>) {
    let backends = gles_backends(state);
    if backends == 0 {
        return;
    }
    let per_backend = match state.renderer_plane_copies {
        Some(copies) => copies,
        None => match probe.and_then(|probe| probe.copies(dmabuf, backends)) {
            Some(copies) => {
                // info!, once per session: whether a GPU client's buffers
                // cost this compositor one fd per plane or two is a question
                // someone sizing its limits asks of the log.
                tracing::info!(
                    copies_per_plane_per_output = copies,
                    "dmabuf: learned how many fds the renderer keeps of each imported plane"
                );
                state.renderer_plane_copies = Some(copies);
                copies
            }
            None => {
                tracing::debug!(
                    "dmabuf: this import could not be measured cleanly; its planes keep the one \
                     renderer copy per plane they were admitted at, and the next is measured"
                );
                return;
            }
        },
    };
    let copies = per_backend.saturating_mul(u8::try_from(backends).unwrap_or(u8::MAX));
    for plane in dmabuf.handles() {
        state.client_fds.settle_copies(plane.as_raw_fd(), copies);
    }
}

/// `copies` spread over `planes` planes and `backends` backends, rounded
/// up and capped at [`MAX_COPIES_PER_PLANE`]. Split out to pin the
/// arithmetic without a renderer.
fn per_plane(copies: usize, planes: usize, backends: usize) -> u8 {
    let per = planes.saturating_mul(backends.max(1));
    let ratio = copies.div_ceil(per.max(1));
    u8::try_from(ratio)
        .unwrap_or(u8::MAX)
        .min(MAX_COPIES_PER_PLANE)
}
/// `fd`'s `(st_dev, st_ino)`.
// `st_dev`/`st_ino` are `u64` on the 64-bit targets scoot builds for, but
// not on every Linux target; see `client_fds/liveness.rs`.
#[allow(clippy::useless_conversion)]
fn identity(fd: BorrowedFd<'_>) -> Option<(u64, u64)> {
    let stat = rustix::fs::fstat(fd).ok()?;
    Some((u64::from(stat.st_dev), u64::from(stat.st_ino)))
}

/// How many fds in this process name the file with identity `(dev, ino)`,
/// or `None` if `/proc/self/fd` cannot be read. An entry that closes while
/// this walks it is simply not counted. Allocates (the directory walk); it
/// runs once per session.
fn fds_naming(dev: u64, ino: u64) -> Option<usize> {
    let entries = std::fs::read_dir("/proc/self/fd").ok()?;
    let count = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
        .filter(|&fd| {
            // SAFETY: borrowed only for the `fstat`, which neither closes nor
            // changes it; a number that closed meanwhile fails with `EBADF`.
            let fd = unsafe { BorrowedFd::borrow_raw(fd) };
            identity(fd) == Some((dev, ino))
        })
        .count();
    Some(count)
}
