//! The release half of explicit sync on the composited path: keep an
//! explicit client buffer until the frame that sampled it is done with it.
//!
//! Smithay signals a buffer's release point when the last reference to its
//! `Buffer` drops (`InnerBuffer::drop`), and on the composited path the only
//! long-lived reference is the surface's own: the moment a client commits
//! the next buffer, the old one's release point is signalled from the CPU.
//! But a composited frame samples client buffers on the GPU *after*
//! `render_frame` returns, and `tty/scanout.rs` hands that frame to KMS with
//! its render fence as `IN_FENCE_FD` rather than waiting on it -- so a client
//! committing between the render and the flip (anything not paced on frame
//! callbacks: mailbox Vulkan, a video player) was told it may reuse a buffer
//! the GPU is still reading. With implicit sync the kernel's reservation
//! fences cover that; with explicit sync nothing does.
//!
//! So `ScanoutPresenter::render_and_queue` hands this hold a clone of every
//! explicit buffer (`ExplicitBuffers`) in each composited frame, tagged with
//! the frame's flip number and paired with its render `SyncPoint`, and the
//! clones drop -- signalling the release points -- when:
//!
//! - **that frame's flip completes** ([`flip_completed`](ReleaseHold::flip_completed)),
//!   the normal path. Every batch at or below the completed number goes,
//!   which also covers a queued frame `DrmCompositor` replaced before it was
//!   ever submitted. Each released batch's render `SyncPoint` is still
//!   waited on first: where the driver takes the render fence as
//!   `IN_FENCE_FD` (or honours the swapchain buffer's implicit fence) a
//!   completed flip means the render finished and the wait returns at once,
//!   but a device that does neither can flip a frame before its render is
//!   done (scoot does not act on `RenderFrameResult::needs_sync`), and the
//!   wait keeps the release exact there too -- at the cost of blocking
//!   exactly when the display itself would have shown an unfinished frame.
//! - **the frame will never flip** -- its queue was refused, the session
//!   paused or reactivated, the compositor was rebuilt on another CRTC, or a
//!   completion errored ([`release_frame`](ReleaseHold::release_frame),
//!   [`release_all`](ReleaseHold::release_all)). Those wait on the batch's
//!   render `SyncPoint` first, which is the exact criterion (the flip was
//!   only ever a non-blocking proxy for it). All are rare paths, and the
//!   wait is normally already satisfied; on a renderer without native fences
//!   `GlesFrame::finish` already `glFinish`ed and the point is signalled.
//! - **more frames are held than can be in flight** (a pending flip and one
//!   queued behind it, [`MAX_FRAMES`]): the oldest is waited out and
//!   dropped, so a display whose flips stop completing cannot grow this
//!   without bound or keep a client's buffers forever.
//!
//! A frame whose primary plane went direct holds nothing here: no client
//! buffer was sampled into the swapchain, and the one on the plane is kept by
//! `DrmCompositor` itself until the frame after it is on screen -- so a
//! direct buffer's release point already follows scanout, not the render.
//!
//! Captures need nothing extra: `Backend::capture` reads the composited
//! swapchain slot (never a client buffer), and the cursor-patch re-render
//! (`render::capture_cursor`) reads its pixels back before returning, inside
//! the same dispatch, so any client buffer it sampled is finished with before
//! a commit could drop it.
//!
//! Generic over the held value so the bookkeeping is unit-testable without a
//! live `wl_buffer`; production holds `smithay`'s `Buffer`.

use smithay::backend::renderer::sync::SyncPoint;

#[cfg(test)]
mod tests;

/// How many composited frames can legitimately be awaiting a flip at once:
/// one submitted to KMS and one queued behind it (`DrmCompositor`'s
/// `pending_frame` and `queued_frame`). A third means one of them was
/// replaced or will never complete.
pub(crate) const MAX_FRAMES: usize = 2;

/// Explicit buffers held for frames still in flight. See the module doc.
#[derive(Debug)]
pub(crate) struct ReleaseHold<T> {
    /// `(flip, buffer)` for every held buffer. Reused across frames, so a
    /// steady stream of frames allocates nothing once it has grown to the
    /// working set.
    held: Vec<(u64, T)>,
    /// `(flip, render sync)` for every frame that holds at least one buffer,
    /// oldest first. At most [`MAX_FRAMES`] entries.
    frames: Vec<(u64, SyncPoint)>,
}

impl<T> Default for ReleaseHold<T> {
    fn default() -> Self {
        Self {
            held: Vec::new(),
            frames: Vec::with_capacity(MAX_FRAMES + 1),
        }
    }
}

impl<T> ReleaseHold<T> {
    /// Holds `buffers` for the composited frame about to be queued as
    /// `flip`, whose GPU work completes at `sync`. Holds nothing (and records
    /// no frame) when `buffers` is empty.
    ///
    /// `flip` must be greater than every flip already held -- the presenter
    /// numbers frames monotonically, and a frame whose queue fails is
    /// released at once ([`release_frame`](Self::release_frame)) so its
    /// number is never reused while held.
    pub(crate) fn hold(
        &mut self,
        flip: u64,
        sync: SyncPoint,
        buffers: impl IntoIterator<Item = T>,
    ) {
        let before = self.held.len();
        self.held
            .extend(buffers.into_iter().map(|buffer| (flip, buffer)));
        if self.held.len() == before {
            return;
        }
        tracing::trace!(
            flip,
            buffers = self.held.len() - before,
            "explicit sync: holding a composited frame's buffers until it is done"
        );
        self.frames.push((flip, sync));
        while self.frames.len() > MAX_FRAMES {
            let (oldest, sync) = self.frames.remove(0);
            wait(&sync);
            self.held.retain(|(held, _)| *held > oldest);
        }
    }

    /// `flip` reached the screen: everything held at or below it is
    /// released, each frame's render waited out first (already finished on
    /// any device that fences its flips -- see the module doc).
    pub(crate) fn flip_completed(&mut self, flip: u64) {
        if self.frames.is_empty() {
            return;
        }
        for (held, sync) in &self.frames {
            if *held <= flip {
                wait(sync);
            }
        }
        let before = self.held.len();
        self.frames.retain(|(held, _)| *held > flip);
        self.held.retain(|(held, _)| *held > flip);
        tracing::trace!(
            flip,
            released = before - self.held.len(),
            "explicit sync: a held frame flipped; its buffers are released"
        );
    }

    /// `flip` will never reach the screen (its queue was refused): waits out
    /// its render and releases what it held.
    pub(crate) fn release_frame(&mut self, flip: u64) {
        let Some(at) = self.frames.iter().position(|(held, _)| *held == flip) else {
            return;
        };
        let (_, sync) = self.frames.remove(at);
        wait(&sync);
        self.held.retain(|(held, _)| *held != flip);
    }

    /// No held frame can be trusted to flip any more (a pause, a
    /// reactivation's drain, a rebuilt compositor, a failed completion):
    /// waits out every held render and releases everything.
    pub(crate) fn release_all(&mut self) {
        for (_, sync) in self.frames.drain(..) {
            wait(&sync);
        }
        self.held.clear();
    }

    /// How many buffers are held. Test-only.
    #[cfg(test)]
    pub(crate) fn held(&self) -> usize {
        self.held.len()
    }

    /// How many frames hold something. Test-only.
    #[cfg(test)]
    pub(crate) fn frames(&self) -> usize {
        self.frames.len()
    }
}

/// Waits for a held frame's render to finish before its buffers are
/// released. A failed wait (`Interrupted`) still releases: holding a
/// client's buffer forever over a fence that cannot be waited on would stall
/// that client for good, which is worse than the one frame the wait guards.
fn wait(sync: &SyncPoint) {
    if sync.wait().is_err() {
        tracing::debug!("explicit sync: a held frame's render fence could not be waited on");
    }
}
