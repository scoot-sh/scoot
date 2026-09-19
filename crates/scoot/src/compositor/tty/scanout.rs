//! The GPU scanout presenter: `DrmCompositor` over a GBM swapchain.
//!
//! The alternative to [`dumb`](super::dumb), and the reason this crate has a
//! `gpu-scanout` Cargo feature at all -- everything reachable from here needs
//! Smithay's `backend_gbm`, which is a link-time dependency on libgbm (see
//! the crate's `Cargo.toml`). Without the feature this module does not exist
//! and `--tty` has exactly the dumb-buffer tier it always had.
//!
//! # What this replaces rather than wraps
//!
//! `DrmCompositor` owns an allocator, a swapchain, buffer ages, page flip and
//! modeset, *and its own `OutputDamageTracker`*. So on this tier none of the
//! dumb tier's machinery is called at all -- not `buffers.rs`'s `BufferPool`
//! or its per-slot ages, not `flip_tracker.rs`, and not `Backend`'s own
//! damage tracker. After the presenter split they are not merely unused here
//! but unreachable: they live behind [`DumbPresenter`](super::dumb::DumbPresenter),
//! which this tier's [`Presenter`](super::Presenter) variant does not hold.
//!
//! One piece is deliberately *shared* rather than replaced, against the
//! letter of the staging note: [`PresentRetries`](super::present_retry::PresentRetries).
//! A refused `queue_frame` is the same hazard here as a refused `page_flip`
//! there -- nothing is in flight, so no completion event will ever retry the
//! frame, and the screen stays stale until unrelated damage arrives -- and
//! the counter is a pure bounded-retry arithmetic with no dumb-buffer
//! coupling whatsoever. Duplicating it would have been worse engineering than
//! reusing it.
//!
//! # Which flip is which
//!
//! `flip_tracker.rs`'s job here is done by `DrmCompositor` itself:
//! [`queue_frame`](DrmCompositor::queue_frame) takes a `user_data` and
//! [`frame_submitted`](DrmCompositor::frame_submitted) hands that exact value
//! back when the vblank for *that* frame arrives. So the session-lock blank
//! confirmation (`session_lock.rs`, and
//! `docs/backlog/resolved/session-lock-vblank-confirm-done.md`) rides on the
//! kernel's own pairing rather than on "at most one flip is out" -- which is
//! not true here, since `queue_frame` may queue behind a pending flip instead
//! of refusing it.
//!
//! The one residual corner is the same one the dumb tier accepts and
//! documents: a *stale* completion for a flip that was issued before a pause
//! or a modeset, arriving after a fresh frame is already pending, is handed
//! back as the fresh frame's number and can confirm a lock up to one vblank
//! early. It needs a discard, a re-present and a still-queued stale
//! completion all inside the lock window, and its worst case is exactly the
//! pre-vblank-confirmation behaviour rather than a new failure mode.
//! Confirming *late* is always safe (the fallback deadline bounds it);
//! confirming early is the one that matters, and one vblank is the bound.

use std::error::Error;

use smithay::backend::allocator::Format as DrmFormat;
use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::drm::compositor::{DrmCompositor, FrameFlags, PrimaryPlaneElement};
use smithay::backend::drm::exporter::gbm::{GbmFramebufferExporter, NodeFilter};
use smithay::backend::drm::{DrmDeviceFd, DrmSurface, PlaneInfo, Planes};
use smithay::backend::renderer::element::RenderElement;
use smithay::backend::renderer::{Bind, Color32F, Renderer, Texture};
use smithay::output::{Output, OutputModeSource};
use smithay::reexports::drm::control::{Mode, crtc};
use smithay::utils::Transform;

use super::present_retry::{self, PresentRetries};

/// The concrete `DrmCompositor` this backend drives.
///
/// `u64` is the per-frame user data: the flip sequence number the
/// session-lock wait matches on (see this module's doc). `DrmDeviceFd` is the
/// cursor-plane device parameter, which is inert here -- `DrmCompositor::new`
/// is passed `gbm: None`, disabling the cursor plane outright (see
/// [`ScanoutPresenter::build`]).
type Compositor =
    DrmCompositor<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, u64, DrmDeviceFd>;

/// Colour formats offered to `DrmCompositor::new`, in order. `Argb8888` first
/// because it is the format every read-back consumer in this compositor
/// already expects (see `render::read_back`); `Xrgb8888` is the same layout
/// with the alpha channel ignored, which some planes prefer.
const COLOR_FORMATS: [Fourcc; 2] = [Fourcc::Argb8888, Fourcc::Xrgb8888];

/// What one frame on this tier did, for `render::draw_frame_scanout` to turn
/// into a [`FrameOutcome`](crate::compositor::render::FrameOutcome).
pub(crate) struct ScanoutFrame {
    /// Whether a frame reached the renderer at all. Only such a frame may
    /// confirm a pending session lock.
    pub(crate) drew: bool,
    /// The flip this frame was queued on, if one was queued. `None` for a
    /// frame with no damage (nothing needed to go out, and the screen already
    /// shows this content) and for a refused commit.
    pub(crate) flip: Option<u64>,
    /// Whether this frame's content actually changed anything on screen --
    /// what `State::frame_serial` counts.
    pub(crate) damaged: bool,
}

/// `DrmCompositor`, plus what is needed to rebuild it on a different CRTC and
/// to bound a refusal streak.
pub(crate) struct ScanoutPresenter {
    compositor: Compositor,
    /// The GBM device the swapchain allocates from, the framebuffer exporter
    /// registers with, and the renderer's EGL display was made on. Kept so a
    /// CRTC switch can rebuild the compositor without re-deriving it.
    gbm: GbmDevice<DrmDeviceFd>,
    /// The renderer's importable dma-buf formats, as
    /// `DrmCompositor::new` intersects them with the plane's. Kept for the
    /// same reason as `gbm`.
    ///
    /// This is *not* stage 4's renderer-derived client format advertisement
    /// and must not be read as pulling it forward: it is a constructor
    /// argument `DrmCompositor::new` requires in order to pick a swapchain
    /// format at all, and it never reaches `zwp_linux_dmabuf_v1`.
    renderer_formats: Vec<DrmFormat>,
    /// Where the compositor reads the mode, scale and transform from. Kept
    /// so a CRTC switch can rebuild the compositor against the same source
    /// -- the real `wl_output` once [`track_output`](Self::track_output) has
    /// run, which is before any frame is drawn. Without this, rebuilding
    /// would have to be handed an `Output` from a call site that has no
    /// business holding one.
    mode_source: OutputModeSource,
    /// The next flip's sequence number. Monotonic for the process's life;
    /// `u64` wrap is not a correctness question (one flip per vblank would
    /// take hundreds of millions of years), the same stance `flip_tracker.rs`
    /// takes.
    next_flip: u64,
    /// How many consecutive commits the kernel has refused -- bounds the
    /// timer-driven retries (see `present_retry.rs`).
    retries: PresentRetries,
    /// Whether the last frame was a refused commit owed a timer-driven retry.
    /// Set only by the refusal arms below, taken only by the render tail (see
    /// [`take_retry_render`](Self::take_retry_render)) -- the same one-writer,
    /// one-taker shape the dumb tier uses, and for the same reason: nothing is
    /// in flight after a refusal, so no completion event can retry it.
    retry_armed: bool,
    /// Whether the swapchain's slots have been freed since the render path
    /// last looked. Set by every path that rebuilds or resizes the swapchain,
    /// taken by `render::draw_frame_scanout`, which uses it to drop the
    /// dma-bufs it exported from those slots (see
    /// `render::scanout::ScanoutBackend::forget_slots`). One writer set, one
    /// taker, so the two sides cannot disagree about whether a cached export
    /// still names a live buffer.
    slots_dropped: bool,
}

impl ScanoutPresenter {
    /// Builds the scanout tier on `surface`, at `size`.
    ///
    /// `size` is the mode's own size: the mode source starts
    /// [`Static`](OutputModeSource::Static) because the `wl_output` does not
    /// exist yet at this point in startup (`tty::init` runs before
    /// `headless::init_named`). [`track_output`](Self::track_output) swaps it
    /// for the real one the moment there is one, and nothing renders in
    /// between.
    pub(super) fn new(
        surface: DrmSurface,
        gbm: GbmDevice<DrmDeviceFd>,
        renderer_formats: Vec<DrmFormat>,
        size: (i32, i32),
    ) -> Result<Self, Box<dyn Error>> {
        let mode_source = OutputModeSource::Static {
            size: size.into(),
            scale: 1.0.into(),
            transform: Transform::Normal,
        };
        let compositor = Self::build(
            &surface_planes(&surface),
            surface,
            &gbm,
            &renderer_formats,
            mode_source.clone(),
        )?;
        Ok(Self {
            compositor,
            gbm,
            renderer_formats,
            mode_source,
            next_flip: 0,
            retries: PresentRetries::new(),
            retry_armed: false,
            slots_dropped: false,
        })
    }

    /// The one `DrmCompositor::new` call, shared by startup and by a CRTC
    /// switch so the two cannot configure it differently.
    ///
    /// Three choices here are stage-3 scope decisions, not defaults:
    ///
    /// - **`planes` is restricted to the primary plane.** `None` would hand
    ///   the compositor every plane the CRTC has; cursor and overlay planes
    ///   are their own piece of work (and the cursor plane would also need a
    ///   `gbm` argument below, which is why that is `None`).
    /// - **`FrameFlags::empty()`, not `DEFAULT`.** `DEFAULT` is
    ///   `ALLOW_SCANOUT`, which lets a client's own buffer be scanned out
    ///   directly on the primary plane instead of being composited into the
    ///   swapchain slot. That is a real optimisation and it is not this
    ///   stage's: it would mean the frame is *not* in the swapchain buffer,
    ///   so `render::scanout`'s capture path -- screenshots and
    ///   `ext-image-copy-capture-v1`, which this project treats as
    ///   first-class -- would silently start returning something that is not
    ///   what is on screen.
    /// - **`cursor_size` is only read when a cursor plane exists**, which it
    ///   does not here; the value is Smithay's own `(64, 64)` convention.
    fn build(
        planes: &Planes,
        surface: DrmSurface,
        gbm: &GbmDevice<DrmDeviceFd>,
        renderer_formats: &[DrmFormat],
        mode_source: OutputModeSource,
    ) -> Result<Compositor, Box<dyn Error>> {
        // `RENDERING | SCANOUT`: the buffers are both drawn into by GLES and
        // handed to the CRTC, so they must satisfy both.
        let allocator = GbmAllocator::new(
            gbm.clone(),
            GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
        );
        // `NodeFilter::None` disables direct scan-out of *client* buffers,
        // which is the same decision `FrameFlags::empty()` makes above, made
        // again at the layer that would have to import them.
        let exporter = GbmFramebufferExporter::new(gbm.clone(), NodeFilter::None);
        let compositor = Compositor::new(
            mode_source,
            surface,
            Some(planes.clone()),
            allocator,
            exporter,
            COLOR_FORMATS,
            renderer_formats.to_vec(),
            (64, 64).into(),
            None,
        )?;
        Ok(compositor)
    }

    /// Points the compositor's mode/scale/transform at the real `wl_output`,
    /// so a later mode change or scale change is followed without a second
    /// place having to remember to mirror it.
    ///
    /// Called once, by `headless::init_named`, immediately after the output
    /// is created -- which is the only moment at which this is both possible
    /// (the output exists) and still free (no frame has been drawn).
    pub(super) fn track_output(&mut self, output: &Output) {
        self.mode_source = output.into();
        self.compositor
            .set_output_mode_source(self.mode_source.clone());
    }

    /// The CRTC this presenter drives.
    pub(super) fn crtc(&self) -> crtc::Handle {
        self.compositor.crtc()
    }

    /// The surface being driven, for the hotplug path's connector/mode moves.
    pub(super) fn surface(&self) -> &DrmSurface {
        self.compositor.surface()
    }

    /// Composites `elements` straight into a swapchain slot and queues that
    /// slot for scan-out.
    ///
    /// `on_frame` is handed the GBM buffer the frame landed in, and only when
    /// the frame actually drew something -- `render::draw_frame_scanout` uses
    /// it to record the dma-buf a capture reads. A frame with no damage
    /// deliberately does not call it: the previous frame is still what is on
    /// screen, so the recorded buffer must stay the previous one.
    ///
    /// An empty frame is the *normal* no-damage case, not a failure: nothing
    /// is queued, no retry is armed and no warning is logged, exactly as the
    /// dumb tier simply never calls `present` when `render_output` reports no
    /// damage.
    pub(crate) fn render_and_queue<R, E>(
        &mut self,
        renderer: &mut R,
        elements: &[E],
        clear_color: Color32F,
        mut on_frame: impl FnMut(&smithay::backend::allocator::gbm::GbmBuffer),
    ) -> ScanoutFrame
    where
        R: Renderer + Bind<Dmabuf>,
        R::TextureId: Texture + 'static,
        E: RenderElement<R>,
    {
        let result =
            match self
                .compositor
                .render_frame(renderer, elements, clear_color, FrameFlags::empty())
            {
                Ok(result) => result,
                Err(error) => {
                    tracing::warn!(%error, "could not render the frame for scanout");
                    return ScanoutFrame {
                        drew: false,
                        flip: None,
                        damaged: false,
                    };
                }
            };
        let damaged = !result.is_empty;
        if damaged && let PrimaryPlaneElement::Swapchain(element) = &result.primary_element {
            on_frame(element.buffer());
        }
        // Dropped before `queue_frame`, per `RenderFrameResult`'s own doc:
        // holding it keeps a swapchain slot out of circulation.
        drop(result);

        if !damaged {
            // Nothing changed, so nothing goes out. The screen already shows
            // this content, so this is not a skipped frame owed a retry --
            // it is the same no-op the dumb tier gets by `render_output`
            // reporting no damage and `present` never being called.
            return ScanoutFrame {
                drew: true,
                flip: None,
                damaged: false,
            };
        }

        let flip = self.next_flip;
        match self.compositor.queue_frame(flip) {
            Ok(()) => {
                self.next_flip = self.next_flip.wrapping_add(1);
                self.retries.succeeded();
                ScanoutFrame {
                    drew: true,
                    flip: Some(flip),
                    damaged: true,
                }
            }
            Err(error) => {
                tracing::warn!(%error, "drm: queueing the frame for scanout failed");
                self.arm_retry();
                ScanoutFrame {
                    drew: true,
                    flip: None,
                    damaged: true,
                }
            }
        }
    }

    /// Settles the flip a vblank just completed, returning whether a render
    /// should be re-triggered and the completed flip's number for the
    /// session-lock wait.
    ///
    /// The number comes from the kernel's own pairing: it is whatever
    /// `queue_frame` was given for the frame that just reached scan-out. A
    /// vblank with nothing pending (a stale completion for a frame a reset
    /// already discarded) answers `None` and confirms nothing.
    ///
    /// An error here is not cosmetic: `frame_submitted` has already taken the
    /// pending frame, and it failed while submitting whatever was queued
    /// behind it -- so that frame, which may be the blanked one a lock is
    /// waiting on, is gone with no completion coming. Treated exactly like a
    /// refused commit: bounded timer-driven retry, and no number, so the lock
    /// falls back to its deadline rather than confirming against a frame that
    /// never scanned out.
    pub(super) fn frame_submitted(&mut self) -> (bool, Option<u64>) {
        match self.compositor.frame_submitted() {
            Ok(flip) => (false, flip),
            Err(error) => {
                tracing::warn!(%error, "drm: a queued frame could not be submitted after the vblank");
                self.arm_retry();
                (std::mem::take(&mut self.retry_armed), None)
            }
        }
    }

    /// Records a refused commit against the bounded retry budget. The
    /// caller's own `warn!` has already said what failed; this decides
    /// whether anything is owed a retry.
    fn arm_retry(&mut self) {
        match self.retries.failed() {
            present_retry::Retry::Arm => self.retry_armed = true,
            present_retry::Retry::GiveUp => tracing::warn!(
                "drm: scanout commits keep failing; leaving the screen as-is \
                 until new damage arrives"
            ),
            present_retry::Retry::Quiet => {}
        }
    }

    /// Takes whether the last frame was a refused commit owed a timer-driven
    /// retry. Read once per frame by the render tail.
    pub(crate) fn take_retry_render(&mut self) -> bool {
        std::mem::take(&mut self.retry_armed)
    }

    /// Takes whether the swapchain's slots have been freed since the render
    /// path last looked (see the field's doc).
    pub(crate) fn take_slots_dropped(&mut self) -> bool {
        std::mem::take(&mut self.slots_dropped)
    }

    /// Re-reads the CRTC's state after a session reactivation, and clears any
    /// frame the pause left pending.
    ///
    /// `reset_state` alone is what Smithay's own `DrmOutputManager::activate`
    /// does, and it is not enough here. It sets `reset_pending` (so the next
    /// frame is a full commit rather than a page flip onto state another VT
    /// may have reconfigured) but it deliberately does **not** touch
    /// `pending_frame` -- and `queue_frame` only *submits* when
    /// `pending_frame` is `None`, queueing behind it otherwise. So a flip
    /// that was still in flight when the session paused, whose vblank the
    /// kernel then never delivers, would leave every subsequent frame queued
    /// and never submitted: a permanently black screen after a VT switch
    /// back, with no error anywhere. That is precisely the failure class
    /// `docs/roadmap/05b-vt-switch-eperm.md` is about, so the drain is not
    /// belt-and-braces.
    ///
    /// The drained frame's number is discarded rather than reported: its
    /// content never reached the screen, so it must not confirm a session
    /// lock. The wait stays, owned by its fallback deadline, until the
    /// post-reactivation render records a fresh flip.
    pub(super) fn reactivate(&mut self) {
        if let Err(error) = self.compositor.reset_state() {
            tracing::warn!(%error, "could not reset drm surface state after reactivation");
        }
        if let Err(error) = self.compositor.frame_submitted() {
            // debug!, not warn!: submitting a frame prepared before the pause
            // onto a CRTC that has just been reset is expected to fail, and
            // the failure is harmless -- `frame_submitted` has already
            // cleared the pending slot, which is the whole point of the call,
            // and `reactivate` unconditionally asks for a fresh render.
            tracing::debug!(%error, "drm: a pre-pause frame could not be submitted; discarding it");
        }
        self.invalidate_scanout();
    }

    /// The scanout bookkeeping shared by reactivation and the hotplug paths:
    /// after either, nothing about what the CRTC is showing can be trusted.
    ///
    /// `reset_buffers` drops every swapchain slot, which is both what makes
    /// the next frame a full redraw (there is no buffer age left to trust)
    /// and what obliges the render side to drop the dma-bufs it exported from
    /// those slots -- hence `slots_dropped`. The refusal streak is reset with
    /// them: a CRTC that has just been reconfigured is new device state, and
    /// the first transient refusal on it must arm a retry rather than answer
    /// `Quiet` off a streak it never earned.
    pub(super) fn invalidate_scanout(&mut self) {
        self.compositor.reset_buffers();
        self.slots_dropped = true;
        self.retries = PresentRetries::new();
    }

    /// Moves the compositor onto a new mode: the surface's pending mode and
    /// the swapchain's size together.
    ///
    /// The hotplug path has already put the mode on the *surface* (see
    /// `hotplug.rs`'s `set_pending`, which also handles the connector/mode
    /// ordering dance a connector change needs). This repeats
    /// `surface.use_mode` -- one more `TEST_ONLY` commit on a path that runs
    /// when a cable moves -- because `DrmCompositor::use_mode` is the only
    /// thing that also resizes the swapchain, and re-deriving that here from
    /// its internals would be a copy of Smithay's business.
    pub(super) fn use_mode(&mut self, mode: Mode) -> bool {
        match self.compositor.use_mode(mode) {
            Ok(()) => {
                self.invalidate_scanout();
                true
            }
            Err(error) => {
                tracing::warn!(%error, "drm: the compositor would not take the new mode");
                false
            }
        }
    }

    /// Rebuilds the compositor on a surface belonging to a different CRTC,
    /// answering whether it took.
    ///
    /// `DrmCompositor` owns its `DrmSurface` by value, so following a
    /// connector onto another CRTC means a new compositor, not a mutated one.
    /// Built first and installed only on success, which is the same property
    /// `hotplug.rs`'s `switch_crtc` relies on for the dumb tier: a failed
    /// switch leaves the live compositor -- and what is on screen -- exactly
    /// as it was, and the candidate surface drops with the failure.
    pub(super) fn adopt_surface(&mut self, surface: DrmSurface) -> bool {
        let planes = surface_planes(&surface);
        match Self::build(
            &planes,
            surface,
            &self.gbm,
            &self.renderer_formats,
            self.mode_source.clone(),
        ) {
            Ok(compositor) => {
                self.compositor = compositor;
                // The old compositor's swapchain drops with it, so every
                // dma-buf the render side exported from it is stale.
                self.slots_dropped = true;
                self.retries = PresentRetries::new();
                true
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    "drm: could not build a scanout compositor on the other crtc"
                );
                false
            }
        }
    }
}

/// The planes `DrmCompositor` may use: this surface's own primary plane and
/// nothing else. See [`ScanoutPresenter::build`] for why cursor and overlay
/// planes are out of scope here.
///
/// The filter is not decoration. `DrmSurface::planes()` reports every primary
/// plane the CRTC could use, and `DrmCompositor` assumes the primary it is
/// given is the one the surface commits against -- handing it a different
/// one would be a plane the surface never claimed.
fn surface_planes(surface: &DrmSurface) -> Planes {
    let primary: Vec<PlaneInfo> = surface
        .planes()
        .primary
        .iter()
        .filter(|plane| plane.handle == surface.plane())
        .cloned()
        .collect();
    Planes {
        primary,
        cursor: Vec::new(),
        overlay: Vec::new(),
    }
}
