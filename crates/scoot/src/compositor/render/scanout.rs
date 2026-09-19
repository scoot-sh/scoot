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
//! [`Captures::forget_slots`] and its one caller). That caller is the
//! only thing that can free a slot, so a stale entry cannot outlive the
//! buffer it names.

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
        // Before anything else can depend on this renderer: a tier that
        // cannot import what this compositor *promises* clients it can
        // import is a tier that kills dmabuf clients, and it must not come
        // up at all.
        if let Some(refused) = first_unimportable(&renderer) {
            return Err(format!(
                "this device's gles renderer cannot import {:?}/{:?}, which \
                 zwp_linux_dmabuf_v1 advertises to every client -- coming up on \
                 it would disconnect them",
                refused.code, refused.modifier
            )
            .into());
        }
        Ok(Self {
            renderer,
            captures: Captures {
                exported: Vec::with_capacity(SLOT_POOL),
                frame: None,
            },
        })
    }

    /// The dma-buf formats this renderer can import, which is what
    /// `DrmCompositor::new` intersects with the primary plane's to pick a
    /// swapchain format.
    ///
    /// Read from the renderer that will actually draw the frames, not from a
    /// probe: a format table that described a *different* EGL context would
    /// be a promise nothing keeps. Deliberately not related to what
    /// `zwp_linux_dmabuf_v1` advertises to clients -- that is stage 4's, and
    /// this value never reaches it.
    pub(crate) fn renderer_formats(&self) -> Vec<smithay::backend::allocator::Format> {
        self.renderer
            .egl_context()
            .dmabuf_render_formats()
            .iter()
            .copied()
            .collect()
    }
}

/// The first format this compositor advertises to dmabuf clients that
/// `renderer` cannot actually import, if any.
///
/// Not a nicety and not stage 4. `zwp_linux_dmabuf_v1`'s feedback tranche is
/// a promise with teeth: a client that allocates from it and then has the
/// import refused is killed outright, because `create_immed`'s only failure
/// reply is a fatal protocol error (see `dmabuf.rs`'s module doc and
/// `docs/backlog/resolved/dmabuf-advertised-but-never-imported-done.md`,
/// where exactly that took down a whole shell). Under `--tty` the renderer
/// has always been pixman, which imports a linear dma-buf by mmapping it and
/// essentially never refuses; this tier is the first thing that routes those
/// imports through GLES, which *can*. So the check is the price of adding the
/// tier, not a feature of it.
///
/// What it deliberately does **not** do is change what is advertised --
/// deriving the tranche from the active renderer is stage 4, and is a
/// different (and larger) change. This only refuses to come up on a
/// configuration where the existing advertisement would be a lie.
fn first_unimportable(renderer: &GlesRenderer) -> Option<smithay::backend::allocator::Format> {
    use smithay::backend::renderer::ImportDma;

    crate::compositor::dmabuf::advertised_formats()
        .find(|format| !renderer.has_dmabuf_format(*format))
}

impl Captures {
    /// The dma-buf a capture should read: the one carrying the most recently
    /// rendered frame, or `None` before the first one.
    ///
    /// `&mut` because Smithay's `Bind` takes its target that way (it may
    /// attach an FBO to it). Nothing on the capture path mutates the frame's
    /// pixels.
    pub(super) fn frame_mut(&mut self) -> Option<&mut Dmabuf> {
        self.frame.as_mut()
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
    pub(super) fn note_frame(&mut self, buffer: &GbmBuffer) {
        let key = std::ptr::from_ref(buffer) as usize;
        if let Some(index) = self.exported.iter().position(|(slot, _)| *slot == key) {
            self.frame = Some(self.exported[index].1.clone());
            return;
        }
        match buffer.export() {
            Ok(dmabuf) => {
                // Bounded: the pool is a cache, not a registry, so the
                // oldest entry goes rather than the vector growing if a
                // future swapchain is deeper than `SLOT_POOL`.
                if self.exported.len() == SLOT_POOL {
                    self.exported.remove(0);
                }
                self.exported.push((key, dmabuf.clone()));
                self.frame = Some(dmabuf);
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    "could not export the scanned-out buffer for capture; \
                     captures will read the previous frame"
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
    }
}
