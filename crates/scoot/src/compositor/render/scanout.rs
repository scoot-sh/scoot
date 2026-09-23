//! The GPU scanout pipeline's renderer half: GLES compositing straight into
//! the buffer the CRTC scans out.
//!
//! This is the *only* tier with no read-back. [`pixman`](super::pixman) and
//! [`gles`](super::gles) both composite into a target in this process's
//! address space and hand the bytes to a presenter; here the target is a
//! `DrmCompositor` swapchain slot -- a GBM buffer already registered as a DRM
//! framebuffer -- so the frame reaches the screen by being flipped, not by
//! being copied.
//!
//! **Which half lives where.** The renderer lives here, in
//! [`Backend`](super::Backend), because every non-frame consumer of a
//! renderer reaches it through `Backend` and nothing else:
//! `Backend::capture` (screenshots and `ext-image-copy-capture-v1`),
//! `Backend::import_dmabuf` and `Backend::cleanup_texture_cache`. The
//! `DrmCompositor` lives in `tty/scanout.rs`, because everything *it* needs
//! (the DRM surface, the CRTC, the connector, session pause/resume, hotplug)
//! is `Tty`'s. `render::draw_frame` is the one place that holds both at once,
//! which is exactly where a frame needs them.
//!
//! # Capturing without a read-back target
//!
//! The other two tiers own a persistent framebuffer a capture can simply be
//! read out of. Here there is no such thing: each frame lands in whichever
//! swapchain slot was free. So [`ScanoutBackend::note_frame`] records the
//! dma-buf that carried the most recent frame, and [`ScanoutBackend::frame`]
//! hands it to `Backend::capture`, which binds it and reads it back exactly
//! as it reads back pixman's image. The content is the screen as of the last
//! frame that actually drew something, which is what a capture wants: a
//! render that produced no damage leaves the previous frame on screen *and*
//! leaves this pointing at it.
//!
//! Exporting a `Dmabuf` is not free (an fd per plane, plus allocation), and
//! a frame path must not do that every frame -- so the exports are pooled per
//! swapchain slot. The pool is keyed by the address of the `GbmBuffer` inside
//! the slot, which is stable for as long as the swapchain is, and is dropped
//! wholesale whenever the swapchain is rebuilt (see
//! [`Captures::forget_slots`] and its one caller).
//!
//! That pointer is only a safe key while *every* path that frees a slot
//! flags it. Rebuild and resize are the obvious ones; the non-obvious one is
//! a failed `render_frame`, which resets the swapchain internally on its way
//! out (`drm/compositor/mod.rs`'s `Err` arm at the pinned rev). Miss one and
//! the allocator will reuse the freed address for the next slot, a cached
//! export aliases it, and a stale frame is served as a current one. Keep
//! `slots_dropped` set on all of them.
//!
//! # What a capture sees when the cursor rides its own plane
//!
//! Steps 1+2 of `docs/backlog/resolved/gpu-scanout-planes-done.md` let Smithay
//! assign the cursor element to a KMS cursor plane and (where the CRTC has
//! overlays but no cursor plane) to an overlay plane (the cursor/overlay
//! frame flags in `tty/scanout.rs`). A plane-assigned element is *not*
//! drawn into the swapchain slot -- it reaches the screen through its own
//! commit -- so the dma-buf recorded here carries the screen *without* the
//! cursor, and every consumer of [`Captures::capture_target`] (IPC
//! screenshots, `ext-image-copy-capture-v1`) shows a cursorless screen on
//! exactly those sessions. Where the cursor stays composited nothing
//! changes: captures keep showing it, as does `screencopy.rs`'s cursor
//! section.
//!
//! That is a semantic change, not a bug, and it is stated here rather than
//! fixed here: compositing the cursor back into the capture would be a second
//! cursor render on a path whose whole point is reading one buffer.
//! It is live-observed on virtio-gpu (captures byte-identical across cursor
//! moves); `paint_cursors=false` stays accepted-and-
//! ignored throughout: the flag never changes which buffer the cursor is
//! drawn into, only whether a session whose cursor rides a plane shows it
//! in captures (it does not) or one whose cursor stays composited does (it
//! does, exactly as before these steps).
//!
//! What a capture can never be missing *without knowing it* is a window.
//! Smithay's overlay assignment only considers elements of kind
//! `ScanoutCandidate` or `Cursor`, and this tree builds every window, popup
//! and layer-shell surface element as `Kind::Unspecified` (only cursor
//! elements -- the compositor's own and a client's cursor surface -- are
//! `Kind::Cursor`). So an overlay plane on this tier can carry at most a
//! cursor -- never a toplevel -- and marking a window a scanout candidate
//! is a separate semantic change, not part of this step. The one way a
//! window could leave the swapchain slot is primary-direct, which is what
//! the next section's mark covers.
//!
//! # What a capture sees when the primary goes direct
//!
//! `tty/scanout.rs` passes `ALLOW_SCANOUT`, and its framebuffer exporter
//! admits client dma-bufs, so a frame *may* land on the primary plane direct
//! instead of in the swapchain slot -- in which case the dma-buf recorded
//! here names the previous composite, which is not what is on screen.
//! Serving it would hand every capture consumer (IPC screenshots,
//! `ext-image-copy-capture-v1`, and through it shell thumbnails and
//! overviews) a stale screen to act on, which for an agent driving this
//! compositor is acting on a screen that is not there.
//!
//! Two halves, never apart:
//!
//! - **Mark.** `tty/scanout.rs` reports the direct arm in its per-frame
//!   outcome (`ScanoutFrame::primary_direct`), and `draw_frame_scanout`
//!   turns it into [`Captures::note_direct`]. The mark says "the recording
//!   is not current"; the stale composite stays in place behind it, and
//!   only a composite that is actually recorded clears it (a forced frame
//!   whose slot cannot be exported leaves it up).
//! - **Force.** Before serving a capture off a stale-or-missing recording,
//!   `State::ensure_scanout_capture_current` arms one composite-only frame
//!   (`tty::scanout::ForceComposite`) and invalidates the swapchain (which
//!   forces the full damage a static screen would otherwise draw nothing
//!   on), then renders it immediately. The forced frame re-records through
//!   the normal path, so the capture reads fresh pixels. It keys on
//!   [`Captures::capture_stale`], which is true for exactly the states
//!   [`Captures::capture_target`] refuses -- the force fires for every
//!   capture that would otherwise fail, and for no other.
//!
//! What cannot be forced -- a session holding no DRM master, where the
//! render draws nothing -- stays refused, loudly: [`Captures::capture_target`]
//! answers [`REFUSE_DIRECT`] on a marked recording instead of serving the
//! stale buffer, and `Backend::capture` reports it. That refusal is transient
//! by construction (the next composite frame clears the mark).
//!
//! **Reachability, stated rather than implied.** The whole sequence -- mark,
//! refusal, force, clear -- is pinned against this code in `scanout/tests.rs`.
//! It has not fired on a shipped build, because no frame goes primary-direct
//! on any machine measured: Smithay only hands the primary plane to a client
//! buffer whose framebuffer `Format` equals the swapchain slot's, and the
//! opaque-fallback fourcc plus the `LINEAR`-vs-implicit modifier make that
//! unequal on an `Argb8888` swapchain (traced and measured in
//! `tty/scanout.rs`'s `FRAME_FLAGS` doc, with the one device shape that
//! could match). It *has* fired live on the dev VM with that gate lifted in
//! an uncommitted experiment. Lifting it for real is the change that makes
//! these halves live.

use std::error::Error;

use smithay::backend::allocator::dmabuf::{AsDmabuf, Dmabuf};
use smithay::backend::allocator::gbm::{GbmBuffer, GbmDevice};
use smithay::backend::drm::DrmDeviceFd;
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::renderer::gles::GlesRenderer;

/// How many exported dma-bufs to keep. `DrmCompositor`'s swapchain holds four
/// slots at the pinned rev (`Swapchain::new`), so four entries make the pool
/// warm after four frames and never grow again. A larger swapchain would
/// simply re-export the oldest slot occasionally rather than misbehave, which
/// is why this is a capacity hint and not an assertion.
const SLOT_POOL: usize = 4;

/// GLES, plus the dma-buf bookkeeping a scanout session needs for captures.
///
/// The two halves are separate fields rather than one flat struct because a
/// frame needs a `&mut` to each *at the same time*: the renderer is what
/// `DrmCompositor::render_frame` draws with, and the capture bookkeeping is
/// what its per-frame callback writes to. Disjoint field borrows make that a
/// plain destructure instead of a dance.
pub(crate) struct ScanoutBackend {
    pub(super) renderer: GlesRenderer,
    pub(super) captures: Captures,
    /// This tier's DRM **render** node, for `zwp_linux_dmabuf_v1`'s
    /// `main_device` -- the device a client should allocate the buffers this
    /// renderer imports against (see `dmabuf.rs`).
    ///
    /// Taken from the GBM device rather than from the EGL display, because on
    /// this tier the EGL display frequently cannot answer: it is created
    /// through `PLATFORM_GBM_KHR`, and its `EGLDevice` need not carry
    /// `EGL_EXT_device_drm` at all. Measured, not assumed -- on the dev VM's
    /// virtio-gpu it carries neither that nor
    /// `EGL_EXT_device_drm_render_node`, while the *same* Mesa answers both
    /// for the offscreen tier's enumerated device.
    ///
    /// `None` where the GBM device is not a DRM node this can reason about,
    /// or has no render node of its own -- the split render/display case
    /// (Apple Silicon: `apple,dcp` owns the CRTCs and has no render node) is
    /// the realistic one. `dmabuf.rs`'s path ladder answers then.
    pub(super) node: Option<libc::dev_t>,
}

/// The dma-bufs a capture reads, and the pool they are exported into once per
/// swapchain slot. See the module doc for why the pool exists and what
/// invalidates it.
#[derive(Default)]
pub(super) struct Captures {
    /// Exported dma-bufs, keyed by the address of the `GbmBuffer` they came
    /// from. Built once per slot and reused.
    exported: Vec<(usize, Dmabuf)>,
    /// The dma-buf carrying the most recently rendered frame -- what a
    /// capture reads. `None` before the first frame.
    frame: Option<Dmabuf>,
    /// Whether the last damaged frame went primary-direct instead of into
    /// the swapchain slot. Set by [`note_direct`](Self::note_direct),
    /// cleared by [`note_frame`](Self::note_frame) and
    /// [`forget_slots`](Self::forget_slots). While set, `frame` still names
    /// the last composite -- which is *not* what is on screen -- so a
    /// capture served now must force a composite frame first (see
    /// `State::ensure_scanout_capture_current`), and one that cannot must
    /// fail loudly rather than serve the stale buffer (see
    /// `Backend::capture`).
    direct: bool,
}

impl ScanoutBackend {
    /// Builds a GLES renderer on `gbm`, the same device the swapchain
    /// allocates from and the framebuffer exporter registers with.
    ///
    /// Deliberately *not* `EGLDevice::enumerate()` the way
    /// [`gles`](super::gles) does. That picks the best device for rendering
    /// in isolation; here the renderer's buffers have to be scanned out by
    /// *this* DRM device, so the device is not a choice -- it is given. The
    /// same construction is what keeps the split render/display case
    /// (Apple Silicon: AGX has the render node, `apple,dcp` owns the CRTCs)
    /// expressible later without a new abstraction, since
    /// `DrmCompositor::new` already takes the allocator and the framebuffer
    /// exporter separately: the allocator's `GbmDevice` would wrap the render
    /// node's fd and the exporter's the display node's. **Untested** -- this
    /// project has no such machine to try it on, and saying so is the point.
    pub(crate) fn new(gbm: &GbmDevice<DrmDeviceFd>) -> Result<Self, Box<dyn Error>> {
        // Before Smithay's first EGL touch: without it that touch panics
        // instead of failing (see `gles::lib_loadable`). An `Err` here
        // reaches `try_scanout`, which falls back to the cpu renderer and
        // dumb buffers the same way it does when the device cannot drive
        // scanout.
        super::gles::lib_loadable(super::gles::LIB_EGL_SONAME)?;
        // SAFETY: `EGLDisplay::new`'s contract is that nothing *else* in this
        // process calls `eglGetPlatformDisplay`/`eglTerminate` behind
        // smithay's back, so smithay's own refcounting of displays stays
        // truthful. scoot only ever reaches EGL through smithay, and only
        // from here and `gles.rs` -- never both in one session.
        let display = unsafe { EGLDisplay::new(gbm.clone()) }?;
        let context = EGLContext::new(&display)?;
        // SAFETY: the context must not be current on another thread. It was
        // created on this thread one line ago and has never been handed
        // anywhere; `GlesRenderer` is neither `Send` nor `Sync` and lives in
        // `State`, which is single-threaded (see `state.rs`).
        let renderer = unsafe { GlesRenderer::new(context) }?;
        Ok(Self {
            renderer,
            captures: Captures {
                exported: Vec::with_capacity(SLOT_POOL),
                frame: None,
                direct: false,
            },
            node: render_node(gbm),
        })
    }

    /// The dma-buf formats this renderer can import, which is what
    /// `DrmCompositor::new` intersects with the primary plane's to pick a
    /// swapchain format.
    ///
    /// Read from the renderer that will actually draw the frames, not from a
    /// probe: a format table that described a *different* EGL context would
    /// be a promise nothing keeps. Deliberately unrelated to what
    /// `zwp_linux_dmabuf_v1` advertises to clients: these are the formats this
    /// renderer can *render into* for scanout, that one is what it can
    /// *import* from a client, and the two sets differ.
    ///
    /// # What used to be here, and why it is gone
    ///
    /// `ScanoutBackend::new` used to refuse to come up at all on a device
    /// whose renderer could not import both formats `dmabuf.rs` advertised --
    /// the price of being the first `--tty` tier to route client imports
    /// through GLES, back when the advertisement was a hard-coded pixman pair
    /// that this tier could contradict. It cannot contradict it any more: the
    /// tranche is now derived from the renderer the session actually built
    /// (`dmabuf::advertise`, run from `headless::init_named` with this very
    /// backend), so a format this renderer cannot import is never advertised
    /// in the first place.
    ///
    /// Keeping both would have left two mechanisms computing one predicate --
    /// "can this renderer import what we advertise" -- and disagreeing about
    /// what to do with it: one narrowing the table, one refusing the tier. The
    /// narrower is the one that cannot kill a client, so it is the one that
    /// stayed.
    ///
    /// **What the guard was also doing, which is worth naming because it is
    /// what makes removing it safe or not.** It was a net under a *false
    /// negative* in that predicate: a renderer wrongly judged unable to import
    /// lost the tier but kept a working dmabuf path, because the session fell
    /// back to pixman. With the guard gone the same false negative ends in no
    /// `zwp_linux_dmabuf_v1` global at all -- GL clients on software
    /// rendering, and a dmabuf-gated shell unable to capture the screen. The
    /// predicate had exactly such a false negative when this was first
    /// written (it required an explicit `LINEAR` entry, which a driver
    /// reporting only `Modifier::Invalid` does not have); that is fixed in
    /// `dmabuf::imports_linear`, and its doc is the place to check before
    /// narrowing the rule again.
    ///
    /// The consequence that remains, stated rather than discovered: on a
    /// device whose GLES renderer really can import neither candidate, the
    /// session now comes up on this tier with no global (GL clients fall back
    /// to `wl_shm`) where before it fell back to pixman and dumb buffers. No
    /// machine this project can reach produces that configuration, and
    /// `dmabuf::advertise` warns loudly when it happens.
    pub(crate) fn renderer_formats(&self) -> Vec<smithay::backend::allocator::Format> {
        self.renderer
            .egl_context()
            .dmabuf_render_formats()
            .iter()
            .copied()
            .collect()
    }
}

/// The DRM render node belonging to the same device as `gbm`, if it has one.
///
/// The *render* node specifically, converted from whichever node the GBM
/// device was opened on (a primary one, under `--tty`): the device
/// `zwp_linux_dmabuf_v1` names is what a client allocates against, and a
/// client that only renders has no business on a primary node. The
/// conversion is a minor-number lookup plus a `stat` of the matching path
/// (`DrmNode::node_with_type`), so it answers `None` rather than guessing
/// when the device has no render node at all.
///
/// Every failure is `None` and none of them is an error worth a log line
/// here: the caller's ladder has a further rung, and `dmabuf.rs` logs which
/// rung actually answered. Startup-only, once per session.
fn render_node(gbm: &GbmDevice<DrmDeviceFd>) -> Option<libc::dev_t> {
    use smithay::backend::drm::{DrmNode, NodeType};

    let node = DrmNode::from_file(gbm).ok()?;
    let render = node.node_with_type(NodeType::Render)?.ok()?;
    Some(render.dev_id())
}

/// `Backend::capture`'s refusal while the recording is marked direct: the
/// last damaged frame went to the primary plane, so the recorded composite is
/// not what is on screen. Transient -- the next composite frame clears it.
pub(super) const REFUSE_DIRECT: &str =
    "the current frame is held for direct scanout; retry once a composite frame lands";

/// `Backend::capture`'s refusal before anything has been recorded (the first
/// frame, or the first after a swapchain rebuild, has not drawn yet).
pub(super) const REFUSE_NOTHING: &str = "nothing has been scanned out yet";

impl Captures {
    /// The dma-buf a capture may read right now, or why none may be.
    ///
    /// The whole of `Backend::capture`'s decision on this tier, here rather
    /// than inline there so the code that runs is the code the tests drive.
    /// A marked recording refuses with [`REFUSE_DIRECT`] even though a
    /// composite is still recorded behind the mark -- serving it would hand
    /// the caller the screen as it was before the direct frame, which is the
    /// harm the mark exists to stop. Both capture callers force a composite
    /// frame first (`State::ensure_scanout_capture_current`), so reaching the
    /// refusal means the force could not draw (no DRM master, a failed
    /// render, a failed export of the forced slot). Nothing recorded at all
    /// refuses with [`REFUSE_NOTHING`] rather than binding an uninitialised
    /// buffer.
    ///
    /// `&mut` because Smithay's `Bind` takes its target that way (it may
    /// attach an FBO to it). Nothing on the capture path mutates the frame's
    /// pixels.
    pub(super) fn capture_target(&mut self) -> Result<&mut Dmabuf, &'static str> {
        if self.direct {
            return Err(REFUSE_DIRECT);
        }
        self.frame.as_mut().ok_or(REFUSE_NOTHING)
    }

    /// Whether a capture served right now would read a stale-or-missing
    /// buffer: nothing recorded yet, or the recording marked direct. The
    /// predicate `State::ensure_scanout_capture_current` keys its forced
    /// composite frame on -- true exactly when [`capture_target`](Self::capture_target)
    /// would refuse (pinned), so the force fires for every capture that
    /// would otherwise fail and for no other. False by construction on every
    /// other tier, which reads a persistent framebuffer instead of a
    /// recording.
    pub(super) fn capture_stale(&self) -> bool {
        self.frame.is_none() || self.direct
    }

    /// Marks the recording direct: the frame just drawn went to the primary
    /// plane, not into the swapchain slot, so the recorded composite is no
    /// longer current.
    ///
    /// Called only for a frame that actually drew (see
    /// `ScanoutFrame::primary_direct`): an undamaged frame left the previous
    /// frame on screen, and must leave the mark alone with it. Keeps the
    /// last composite in place -- a capture served before the forced
    /// composite lands fails on the mark, it never reads the stale buffer
    /// as current.
    pub(super) fn note_direct(&mut self) {
        self.direct = true;
    }

    /// Records the swapchain slot a frame was just rendered into, so a later
    /// capture can read it back.
    ///
    /// Called only for a frame that actually drew: one that produced no
    /// damage left the *previous* frame on screen, and must leave this
    /// pointing there too.
    ///
    /// A failed export is logged and leaves the previous frame recorded
    /// rather than clearing it. That is the lesser of the two wrongs: a
    /// capture then returns the frame before this one (stale by one frame)
    /// instead of failing outright, and this cannot happen in a steady state
    /// anyway -- the slot was exported successfully by `render_frame` itself
    /// moments earlier to get its DRM framebuffer.
    ///
    /// A failed export also leaves a direct *mark* in place, and that half
    /// is not a lesser wrong but the rule: the mark says the recorded
    /// composite predates what is on screen, and a forced composite whose
    /// slot could not be exported has not changed that. Clearing it would
    /// serve the pre-direct screen as current.
    pub(super) fn note_frame(&mut self, buffer: &GbmBuffer) {
        self.record(std::ptr::from_ref(buffer) as usize, || buffer.export());
    }

    /// [`note_frame`](Self::note_frame)'s body, over the slot's pool key and
    /// its export: the pool hit, the bounded miss and the failure all live
    /// here, and `note_frame` only supplies the two things that need a live
    /// `GbmBuffer`. Split so the recording's transitions are driven by tests
    /// through the code that runs, not a copy of it.
    fn record<E: std::fmt::Display>(
        &mut self,
        key: usize,
        export: impl FnOnce() -> Result<Dmabuf, E>,
    ) {
        if let Some(index) = self.exported.iter().position(|(slot, _)| *slot == key) {
            self.frame = Some(self.exported[index].1.clone());
            self.direct = false;
            return;
        }
        match export() {
            Ok(dmabuf) => {
                // Bounded: the pool is a cache, not a registry, so the
                // oldest entry goes rather than the vector growing if a
                // future swapchain is deeper than `SLOT_POOL`.
                if self.exported.len() == SLOT_POOL {
                    self.exported.remove(0);
                }
                self.exported.push((key, dmabuf.clone()));
                self.frame = Some(dmabuf);
                self.direct = false;
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    direct = self.direct,
                    "could not export the scanned-out buffer for capture; \
                     captures will read the previous composite, or refuse \
                     with a retry while a direct frame is on screen"
                );
            }
        }
    }

    /// Drops every exported dma-buf, because the swapchain that owned the
    /// slots they came from is gone.
    ///
    /// The *only* thing that frees a swapchain slot is the `DrmCompositor`
    /// rebuilding or resizing its swapchain, and `draw_frame_scanout` takes
    /// that fact straight off the presenter that did it (see
    /// `tty::scanout::ScanoutPresenter::take_slots_dropped`) rather than
    /// keeping a second copy of it here. Without this, a freshly allocated
    /// slot could land at a freed slot's address and be served a dma-buf
    /// holding an older frame at an older size.
    pub(super) fn forget_slots(&mut self) {
        self.exported.clear();
        self.frame = None;
        self.direct = false;
    }
}

#[cfg(test)]
mod tests;
