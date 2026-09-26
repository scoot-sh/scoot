//! The dumb-buffer presenter: `--tty` scanout with no GPU involved at all.
//!
//! This is the tier `--tty` runs by default and the only one that needs no
//! GPU stack, no libgbm and no EGL: `render.rs` composites the frame on the
//! CPU (or offscreen with GLES, which is why the read-back exists), hands the
//! damaged region's pixels here, and [`DumbPresenter::present`] memcpys them
//! into a free DRM dumb buffer and flips it onto the CRTC.
//!
//! Everything in this module is *specific to that transport*. The pieces it
//! owns -- the two dumb buffers and their per-slot ages
//! ([`buffers`](super::buffers)), the in-flight flip's sequence number
//! ([`flip_tracker`](super::flip_tracker)) and the bounded retry counter for a
//! refused commit ([`present_retry`](super::present_retry)) -- exist only
//! because a CPU-composited frame has to be copied into a scanout buffer and
//! flipped by hand. A presenter that composites *into* its scanout buffer has
//! no use for any of them, which is why they live behind this type rather
//! than on [`Tty`](super::Tty) itself.
//!
//! What stays on `Tty` is what is true of the session regardless of how it
//! presents: the libseat session, the DRM device, which connector and mode is
//! being driven, whether DRM master is held (`active`) and whether the
//! session is paused (`session_paused`). Those two in particular are *not*
//! the same question and never have been -- see their docs, and
//! `docs/roadmap/05b-vt-switch-eperm.md` for what conflating them cost.

use smithay::backend::drm::{DrmSurface, PlaneConfig, PlaneState};
use smithay::reexports::drm::control::crtc;
use smithay::utils::{Buffer as BufferSpace, Physical, Rectangle, Size, Transform};

use super::buffers::BufferPool;
use super::flip_tracker::FlipTracker;
use super::present_retry::{self, PresentRetries};

/// The DRM surface this tier flips, the dumb buffers it flips, and the
/// bookkeeping that keeps the two in step.
pub(super) struct DumbPresenter {
    surface: DrmSurface,
    buffers: BufferPool,
    /// Set right after a successful `commit`/`page_flip`, cleared on the
    /// matching `VBlank`. [`present`](Self::present) skips (setting
    /// `present_skipped` instead of blocking) rather than flip again while
    /// this is set -- flipping while a previous flip is still pending fails
    /// with EBUSY.
    ///
    /// This is the presenter's half of the session-lock vblank wait (see
    /// `session_lock.rs`): the number identifies *which* flip is out, so a
    /// completion can be matched against the flip that actually carries the
    /// blanked frame rather than against merely "a flip finished". One field
    /// for both facts -- `is_busy()` is "a flip is in flight" -- so the two
    /// can never disagree.
    flips: FlipTracker,
    /// `true` for the very first frame and again right after a session
    /// reactivation, when the CRTC's state is unknown and only a full
    /// modeset (`commit`), not a `page_flip`, is safe to issue.
    needs_modeset: bool,
    /// Which buffer slot is currently scanned out (or mid-flip to), so the
    /// next flip knows which slot becomes free once *this* flip's `VBlank`
    /// confirms the previous one is off screen.
    showing: Option<usize>,
    /// The slot that will become free on the next `VBlank`, i.e. whatever
    /// `showing` held immediately before the in-flight flip was issued.
    pending_free: Option<usize>,
    /// Mirrors `nested::Host`'s `present_skipped`: set when
    /// [`present`](Self::present) had a frame ready but couldn't flip (a flip
    /// already in flight, or no buffer slot free). Checked on the next
    /// `VBlank` so a skipped frame doesn't leave the screen stale until some
    /// unrelated redraw happens to trigger another one.
    present_skipped: bool,
    /// Whether the last [`present`](Self::present) was a refused
    /// commit/page-flip owed a timer-driven retry. Two fields rather than one
    /// shared with `present_skipped` deliberately: a skip owned by a
    /// completion event (a `VBlank` is owed) and a refusal owned by the frame
    /// timer (nothing is in flight, so no event will ever arrive) are
    /// different facts with different consumers, and sharing a flag between
    /// them would be the fact-in-two-places split this project treats as its
    /// own bug class. Set only by the commit-failure arm, taken only by the
    /// render tail (see [`take_retry_render`](Self::take_retry_render)).
    retry_armed: bool,
    /// How many consecutive flips the kernel has refused -- bounds the
    /// timer-driven retries above (see `present_retry.rs`).
    retries: PresentRetries,
}

impl DumbPresenter {
    /// Wraps a freshly created surface and its buffer pool. `needs_modeset`
    /// starts `true`: the very first frame must be a full `commit`, never a
    /// `page_flip` onto a CRTC whose state this process has not set.
    pub(super) fn new(surface: DrmSurface, buffers: BufferPool) -> Self {
        Self {
            surface,
            buffers,
            flips: FlipTracker::new(),
            needs_modeset: true,
            showing: None,
            pending_free: None,
            present_skipped: false,
            retry_armed: false,
            retries: PresentRetries::new(),
        }
    }

    /// The surface being flipped. Read by the hotplug path, which points it
    /// at a new connector/mode (see `hotplug.rs`'s `set_pending`).
    pub(super) fn surface(&self) -> &DrmSurface {
        &self.surface
    }

    /// The CRTC this presenter drives -- the gamma LUT's CRTC, and what a
    /// `DrmEvent::VBlank` is matched against.
    pub(super) fn crtc(&self) -> crtc::Handle {
        self.surface.crtc()
    }

    /// The buffer age `render::draw_frame_with` should pass `render_output`
    /// for this frame -- see `buffers.rs`'s module doc on why it isn't always
    /// the same value, and `BufferPool::next_age`'s doc for what it means.
    /// A pure peek: pair every call with
    /// [`advance_generation`](Self::advance_generation) once the render it was
    /// used for has actually happened *and reported damage* -- an
    /// empty-damage render freezes Smithay's history, so advancing past it
    /// desyncs the next frame (see `BufferPool::advance_generation`'s doc).
    pub(super) fn next_buffer_age(&self) -> usize {
        self.buffers.next_age()
    }

    /// Must be called exactly once per `render_output` call this presenter's
    /// [`next_buffer_age`](Self::next_buffer_age) was used for that reported
    /// damage -- see `BufferPool::advance_generation`'s doc for why a
    /// damage-free render must not advance, and why this can't be folded
    /// into [`present`](Self::present) itself (a damaging render whose
    /// damage ends up written nowhere still consumed history).
    pub(super) fn advance_generation(&mut self) {
        self.buffers.advance_generation();
    }

    /// Copies an already-rendered frame's `region` into a free dumb buffer
    /// and scans it out. See [`Tty::present`](super::Tty::present), the only
    /// caller, for the contract -- it owns the two guards that are facts
    /// about the session rather than about this transport (DRM master held,
    /// and the frame matching the mode currently being scanned out) and this
    /// owns the rest.
    ///
    /// Returns the issued flip's sequence number (see `flip_tracker.rs`), or
    /// `None` when no flip went out: a previous flip hasn't been confirmed by
    /// a `VBlank` yet (flipping again before that would fail with EBUSY), no
    /// buffer slot was free, or the commit itself failed.
    pub(super) fn present(
        &mut self,
        pixels: &[u8],
        region: Rectangle<i32, Physical>,
        frame_size: (i32, i32),
    ) -> Option<u64> {
        if self.flips.is_busy() {
            self.present_skipped = true;
            return None;
        }
        let Some((index, fb)) = self.buffers.write_region(pixels, region) else {
            // Unlike the in-flight skip above -- an ordinary, frequent,
            // harmless throttle; exactly one slot is always free whenever a
            // flip isn't in flight -- reaching here means neither slot was
            // free even though no flip is pending, which should never
            // happen in normal operation. It means either a buffer-freeing
            // bug leaked a slot (this is the failure mode a prior review
            // flagged as running silently forever once both slots are
            // stuck busy) or `write_region` failed to map a dumb buffer
            // (see its own log line in buffers.rs). warn!, not debug!: this is a
            // bug signal, not routine throttling, so it's fine for it to
            // repeat on every subsequent present() for as long as it lasts.
            tracing::warn!(
                "drm: present skipped, no free buffer slot (both slots busy \
                 with no flip pending)"
            );
            self.present_skipped = true;
            return None;
        };
        self.present_skipped = false;

        let src_size: Size<i32, BufferSpace> = frame_size.into();
        let dst_size: Size<i32, Physical> = frame_size.into();
        let plane_state = PlaneState {
            handle: self.surface.plane(),
            config: Some(PlaneConfig {
                src: Rectangle::from_size(src_size).to_f64(),
                dst: Rectangle::from_size(dst_size),
                transform: Transform::Normal,
                alpha: 1.0,
                damage_clips: None,
                fb,
                fence: None,
            }),
        };

        let result = if self.needs_modeset {
            // info!, not debug!: a modeset is rare (first frame, or right
            // after a session reactivation) and is exactly the event
            // pitfall #2's verification depends on being able to grep for
            // at the default log level -- see the commit introducing this
            // backend.
            tracing::info!("drm: modeset (full commit)");
            self.surface.commit([plane_state], true)
        } else {
            // debug!, not info!: an ordinary page flip happens on every
            // redraw (a keystroke, a cursor blink) -- once cursor
            // rendering exists this could be well over 100 times a second,
            // and logging that at info! would drown out everything else at
            // the default level for no benefit once the modeset/page-flip
            // distinction above has already been proven to work.
            tracing::debug!("drm: page flip");
            self.surface.page_flip([plane_state], true)
        };
        match result {
            Ok(()) => {
                self.needs_modeset = false;
                let seq = self.flips.issued();
                // Whatever was showing before this flip becomes free once
                // this flip's VBlank confirms it's off screen.
                self.pending_free = self.showing.replace(index);
                self.retries.succeeded();
                Some(seq)
            }
            Err(error) => {
                tracing::warn!(%error, "drm commit/page flip failed");
                // Undo the write above -- this slot was never actually
                // sent to the CRTC, so it must not stay marked busy -- and
                // unvouch its age: the pixels reached the slot but never
                // scanout, and the retry must fully redraw rather than
                // trust the fresh `last_written` the copy stored (see
                // `BufferPool::note_write_failed`).
                self.buffers.note_write_failed(index);
                // This frame was rendered and never shown, so the screen is
                // stale by exactly the amount that was damaged -- but unlike
                // the two skips above, no completion event can retry it:
                // nothing is in flight, so no `VBlank` will ever arrive to
                // consume `present_skipped`. Arm a timer-driven retry
                // directly instead (bounded: a device that keeps refusing
                // must not pin the loop at full redraws).
                //
                // Newly reachable rather than newly wrong: `hotplug.rs`'s
                // `invalidate_scanout` discards the in-flight flip while one
                // really may still be out (see its doc for why that
                // is the safer of the two mistakes), which can put one
                // EBUSY-rejected flip between the hotplug and the first
                // frame at the new mode. Self-limiting: the retry either
                // issues (resetting the streak) or exhausts its bound and
                // goes quiet until genuine damage arrives.
                match self.retries.failed() {
                    present_retry::Retry::Arm => {
                        self.retry_armed = true;
                    }
                    present_retry::Retry::GiveUp => {
                        tracing::warn!(
                            "drm: commit/page flip keeps failing; leaving scanout \
                             as-is until new damage arrives"
                        );
                    }
                    present_retry::Retry::Quiet => {}
                }
                None
            }
        }
    }

    /// Whether a flip this presenter issued has not been confirmed yet --
    /// its `VBlank` is still owed. Read when a hotplug drops this head, so a
    /// head built on the same CRTC does not mistake that late event for its
    /// own (see `Tty::stale_vblanks`).
    pub(super) fn flip_in_flight(&self) -> bool {
        self.flips.is_busy()
    }

    /// Takes whether the last [`present`](Self::present) was a refused flip
    /// owed a timer-driven retry (see `present_retry.rs`). Read once per frame
    /// by the render tail, which re-arms the frame timer for it -- the only
    /// consumer, since a refused flip has no completion event coming.
    pub(super) fn take_retry_render(&mut self) -> bool {
        std::mem::take(&mut self.retry_armed)
    }

    /// Whatever flip was in flight is done -- one way (confirmed by a
    /// `VBlank`) or another (its completion is now untrackable, reported as
    /// an `Error` instead) -- so the buffer slot it was about to free
    /// (`pending_free`) is safe, and necessary, to free either way; nothing
    /// else in this module will ever free that slot on our behalf. One method
    /// specifically so the two call sites can't drift the way they did before
    /// (`on_vblank` freed `pending_free`, the `Error` arm didn't, and the
    /// CRTC-mismatch check in `on_vblank` has no equivalent need here -- a
    /// `DrmEvent::Error` isn't scoped to a crtc).
    ///
    /// Returns whether a render should be re-triggered because a previous
    /// `present()` had been skipped, plus the finished flip's number for the
    /// session-lock wait. The `Error` arm's caller deliberately drops the
    /// number: an error means the completion is untrackable, so it must not
    /// confirm a lock -- the fallback deadline owns that wait instead (see
    /// `session_lock.rs`).
    pub(super) fn flip_settled(&mut self) -> (bool, Option<u64>) {
        let completed = self.flips.settled();
        if let Some(index) = self.pending_free.take() {
            self.buffers.mark_free(index);
        }
        (std::mem::take(&mut self.present_skipped), completed)
    }

    /// Forgets the in-flight flip without a completion -- the pause path,
    /// where the device is gone with the session and no completion will
    /// arrive for whatever was out. Its number must not linger to match a
    /// lock wait recorded after it (see `flip_tracker.rs`). The wait itself
    /// stays, owned by the fallback deadline until the switch back
    /// re-renders.
    pub(super) fn discard_flip(&mut self) {
        self.flips.discard();
    }

    /// Re-reads the CRTC's state into the surface after a session
    /// reactivation. See `Tty::reactivate`, the only caller, for why this is
    /// attempted even when `drm.activate` itself failed.
    pub(super) fn reset_state(&mut self) -> Result<(), smithay::backend::drm::DrmError> {
        self.surface.reset_state()
    }

    /// Installs a surface built on a different CRTC (see `hotplug.rs`'s
    /// `switch_crtc`). The old surface -- and with it its primary-plane claim
    /// -- drops here, once the replacement has been proven.
    pub(super) fn adopt_surface(&mut self, surface: DrmSurface) {
        self.surface = surface;
    }

    /// Installs a pool allocated for a new mode size, or keeps the current
    /// one when the size did not change and `buffers` is `None` (see
    /// `hotplug.rs`'s `retarget`).
    ///
    /// The old pool -- and the framebuffers in it, one of which the CRTC may
    /// still be scanning out -- is dropped here. The kernel handles a
    /// framebuffer removed while active by blanking the plane, which is what
    /// a mode change does anyway; the full modeset
    /// [`invalidate_scanout`](Self::invalidate_scanout) arms is what brings it
    /// back, on a buffer that is the right size for the new mode.
    pub(super) fn adopt_buffers(&mut self, buffers: Option<BufferPool>) {
        if let Some(buffers) = buffers {
            self.buffers = buffers;
        }
    }

    /// Starts the refusal streak over. A new CRTC is new device state: the
    /// old connector's refusal streak (if any) says nothing about the new
    /// one, so the first transient refusal on it must arm a retry rather than
    /// answer `Quiet` off a streak it never earned.
    pub(super) fn reset_retries(&mut self) {
        self.retries = PresentRetries::new();
    }

    /// The scanout bookkeeping shared by `Tty::reactivate` and `hotplug.rs`'s
    /// `retarget`/`switch_crtc`: after any of them, nothing this pool records
    /// can be trusted to describe what the CRTC is showing, and only a full
    /// modeset -- not a page flip onto state that may have been reconfigured
    /// behind us -- is safe to issue next.
    ///
    /// `flips` is discarded even though a flip really may still be in
    /// flight. Both directions have a cost and they are not symmetric:
    /// leaving it armed when the flip's `VBlank` never arrives (its
    /// framebuffer having been destroyed, or its CRTC re-modeset underneath
    /// it) freezes the screen permanently with no error anywhere, while
    /// discarding it costs at worst one rejected flip, which `present` logs
    /// and retries from the frame timer (`retry_armed` -- see its error arm;
    /// no `VBlank` is owed since nothing is in flight). Discarding
    /// also retires the number a session-lock wait may have recorded for
    /// that flip, so a late vblank for it cannot confirm a lock whose
    /// blanked frame never scanned out -- the wait stays, owned by the
    /// fallback deadline, until the next render records a fresh flip.
    ///
    /// `mark_all_free` likewise frees the slot the CRTC may still be
    /// scanning out, so the next `write_region` can write into live scanout
    /// -- a torn frame, in principle. Accepted, and bounded to nothing a
    /// user can see: `needs_modeset` is set in the same breath, so the next
    /// `present` issues a full `commit` rather than a page flip, and every
    /// caller of this is already on a path that blanks the screen (a VT
    /// switch back, or a modeset onto a connector that just changed). The
    /// alternative -- keeping the showing slot busy -- reintroduces exactly
    /// the "both slots stuck, nothing ever flips again" state `present`'s
    /// own warning exists for, on a path where nothing can vouch for what
    /// the CRTC is holding.
    pub(super) fn invalidate_scanout(&mut self) {
        self.flips.discard();
        self.needs_modeset = true;
        self.buffers.mark_all_free();
        self.buffers.invalidate_ages();
        self.showing = None;
        self.pending_free = None;
    }
}
