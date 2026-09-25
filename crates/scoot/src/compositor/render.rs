//! The renderer seam: what draws a frame, and what a frame's pixels are
//! handed to afterwards.
//!
//! All three backends -- `--headless`, `--nested` and `--tty` -- draw the
//! same frame the same way and differ only in what they do with it
//! afterwards (nothing, a host surface, a DRM scanout). [`Backend`] is the
//! *renderer* half of that: it owns whatever composites the frame and the
//! target it composites into, while `headless.rs`'s `State::render` owns the
//! frame's lifecycle around it (when to draw, lock confirmation, presentation
//! feedback, frame callbacks) and `nested.rs`/`tty/mod.rs` own the handoff.
//!
//! # Why an enum and not a generic `State`
//!
//! [`Backend`] dispatches on an enum once per frame rather than making the
//! render path generic over the renderer. The dispatch is per *frame*, not
//! per element, so it costs nothing at 60Hz -- whereas a generic `State`
//! would push `Renderer + ImportAll + ImportMem + Bind<_> + ExportMem` bounds
//! through every caller of `render()` and force a second monomorphised copy
//! of the whole frame lifecycle for each renderer.
//!
//! What genuinely has to be generic is only the part that *touches* the
//! renderer: [`draw_frame_with`] (bind, gather, draw, read back) and the
//! element gathering in [`elements`]. Those were mostly generic already --
//! `cursor.rs`, `session_lock.rs`'s `lock_elements` and `decorations.rs` have
//! never named a concrete renderer -- so the seam adds one generic function
//! and one generic enum, not a generic compositor.
//!
//! # What an implementation brings, and what it inherits
//!
//! Damage tracking and the framebuffer's size are renderer-agnostic and live
//! on [`Backend`] itself, so an implementation brings only a renderer and a
//! target -- [`pixman::PixmanBackend`] (the default) and
//! [`gles::GlesBackend`] (opt-in, `--renderer gles`) are each exactly that
//! pair -- and inherits the rest. The three things one has to satisfy are the
//! bounds on [`draw_frame_with`]: import client buffers ([`ImportAll`] +
//! [`ImportMem`]), bind its own target ([`Bind`]), and read that target back
//! to main memory ([`ExportMem`]). Those bounds were written for pixman alone
//! and took the GLES renderer unchanged.
//!
//! # The read-back's orientation, and the trap in it
//!
//! [`read_back`] hands out the framebuffer's bytes exactly as the renderer
//! laid them out, and **deliberately does not consult
//! `TextureMapping::flipped()`** -- see its own doc for why that is a
//! correctness requirement and not an oversight.

use std::error::Error;
use std::fmt;

#[cfg(feature = "gpu-scanout")]
use std::time::Instant;

#[cfg(feature = "gpu-scanout")]
use scoot_core::OutputId;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Format, Fourcc};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::{
    Bind, Color32F, ExportMem, ImportAll, ImportDma, ImportMem, Renderer, Texture,
};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Buffer, Physical, Rectangle};

use crate::cli::RendererKind;

use super::State;
use super::tty::Tty;
use elements::{Elements, FrameContext, ring_elements};
use gles::{GlesBackend, GlesDevice};
use pixman::PixmanBackend;

mod capture_cursor;
mod elements;
mod gles;
mod pixman;
#[cfg(feature = "gpu-scanout")]
mod primary_direct;
#[cfg(feature = "gpu-scanout")]
mod scanout;

#[cfg(feature = "gpu-scanout")]
pub(crate) use capture_cursor::Plane;
pub(crate) use capture_cursor::{CursorInFrame, CursorPatch};
#[cfg(all(test, feature = "gpu-scanout"))]
pub(crate) use primary_direct::PrimaryDirect;
#[cfg(feature = "gpu-scanout")]
pub(crate) use scanout::ScanoutBackend;

/// The GPU scanout renderer travelling from `tty::init` to [`Backend::new`].
///
/// It has to travel, rather than being built where every other renderer is,
/// because of one ordering fact: `DrmCompositor::new` needs the renderer's
/// importable dma-buf formats to pick a swapchain format, and `tty::init`
/// runs before the `wl_output` (and therefore `Backend`) exists. So the
/// renderer is built with the `DrmCompositor` and handed forward.
///
/// A struct with one optional field rather than a bare `Option`, so the whole
/// signature chain compiles identically with and without the `gpu-scanout`
/// feature: without it there is no field, `default()` is the only value, and
/// the tier does not exist.
#[derive(Default)]
pub(crate) struct ScanoutHandoff {
    #[cfg(feature = "gpu-scanout")]
    pub(crate) backend: Option<Box<ScanoutBackend>>,
}

/// Which renderer this session composites with: the flag, else the config
/// file, else the default.
///
/// `flag` is `--renderer`, `file` is `[renderer] backend`, and an explicit
/// flag beats the file the way `--gpu` beats `[tty] gpu` -- including
/// `--renderer pixman`, which is why both are `Option` rather than a resolved
/// value (see [`CompositorOptions::renderer`](crate::cli::CompositorOptions)).
///
/// `tty` is the one case that can override both, and only in a build without
/// the `gpu-scanout` feature. With the feature, `--renderer gles` under
/// `--tty` selects the GPU scanout tier (`tty/scanout.rs`): the frame is
/// composited straight into the buffer the CRTC scans out, with no read-back
/// and no memcpy. Without it, the only GLES pipeline that exists is the
/// offscreen one, which under `--tty` would mean rendering on the GPU only to
/// copy every frame back to the CPU and memcpy it into a dumb buffer --
/// strictly worse than compositing there in the first place. So that build
/// warns and keeps pixman rather than silently accepting a slower session.
///
/// A warning, not a startup error, and that asymmetry with
/// `--headless`/`--nested` is deliberate: on `--tty` scoot *is* the session,
/// so a refusal to start is a lockout (see `config.rs`'s module doc). The
/// second place the same rule applies is `tty::init`, which falls back the
/// same way when the feature is present but the device cannot drive the tier.
pub(super) fn resolve(
    flag: Option<RendererKind>,
    file: Option<RendererKind>,
    tty: bool,
) -> RendererKind {
    let (chosen, warning) = resolve_with(flag, file, tty, cfg!(feature = "gpu-scanout"));
    if let Some(warning) = warning {
        tracing::warn!("{warning}");
    }
    chosen
}

/// [`resolve`]'s decision, with the build-time fact spelled out as an
/// argument so both answers are unit-testable from either build.
///
/// Returns the warning text rather than logging it, so a test can pin *which*
/// refusal happened: "this build has no scanout tier" and "this device cannot
/// drive it" are different problems with different fixes, and a user reading
/// one must not be handed the other's wording.
fn resolve_with(
    flag: Option<RendererKind>,
    file: Option<RendererKind>,
    tty: bool,
    scanout_available: bool,
) -> (RendererKind, Option<&'static str>) {
    let chosen = flag.or(file).unwrap_or_default();
    if tty && chosen == RendererKind::Gles && !scanout_available {
        return (
            RendererKind::Pixman,
            Some(
                "this build has no gpu scanout tier (it was built without the \
                 `gpu-scanout` Cargo feature), and the offscreen gles pipeline \
                 would be slower than the cpu renderer under --tty; using pixman",
            ),
        );
    }
    (chosen, None)
}

/// What draws this session's frames, and what it draws into.
///
/// Built once by `headless::init_named` (and `headless::add_output`), and
/// resized by `State::resize_output` -- in place where the pipeline can be
/// ([`Backend::resize_in_place`]), rebuilt where it cannot. Lives in
/// `State::backends`, keyed by output, and is `take`n for the duration of a
/// frame so the render path can hold `&mut State` and `&mut` the renderer at
/// the same time.
pub struct Backend {
    /// Which renderer is drawing, and the target it draws into.
    pipeline: Pipeline,
    /// What changed since the last frame, per buffer age. Renderer-agnostic
    /// (it tracks element geometry, not pixels), so it lives here rather
    /// than inside [`Pipeline`] -- a second implementation inherits it.
    damage: OutputDamageTracker,
    /// The render target's size in *physical* pixels, which is also the size
    /// a capture client's buffer has to match (see `screencopy.rs`). Kept
    /// beside the target rather than re-derived from it, so every consumer
    /// reads the one number the target was actually built at.
    size: (i32, i32),
    /// What the persistent framebuffer holds of the cursor as of the last
    /// frame drawn into it, for the capture path to reconcile with what a
    /// capture asked for (see `capture_cursor.rs`). Written by
    /// [`draw_frame_with`] on every frame it draws; the scanout tier keeps
    /// its own beside its swapchain recording instead
    /// ([`Backend::cursor_in_frame`] reads whichever applies), so this stays
    /// at its default there.
    cursor: CursorInFrame,
    /// Pixel buffers of cursor regions already written into a capture,
    /// kept for the next one (`capture_cursor.rs`'s `Backend::recycle_patch`):
    /// at most two, so a capture stream re-renders its region into the same
    /// memory frame after frame.
    patch_pixels: Vec<Vec<u8>>,
}

/// What a renderer can import, in the shape the `zwp_linux_dmabuf_v1`
/// advertisement is derived from ([`Backend::dmabuf_import_set`]).
pub(super) enum ImportSet {
    /// The renderer `mmap`s a dma-buf and composites out of the mapping
    /// (pixman): only a single-plane `LINEAR` buffer can work, so the
    /// advertisement is `dmabuf.rs`'s fixed candidates, narrowed by
    /// [`Backend::imports_dmabuf_format`].
    CpuMapped,
    /// The renderer hands a dma-buf to its driver (both GLES tiers): every
    /// `{fourcc, modifier}` the driver reported it imports, in the driver's
    /// order, with Smithay's unconditional `Modifier::Invalid` entries still
    /// in it -- `dmabuf.rs::driver_tranche` is what decides which of those
    /// are a promise.
    Driver(FormatSet),
}

/// What [`Backend::resize_in_place`] did. Whatever the answer, the backend
/// is usable: only [`Resized`](InPlace::Resized) changed anything.
pub(super) enum InPlace {
    /// The target is at the new size, on the same renderer.
    Resized,
    /// This pipeline has no in-place path here, and nothing changed. For
    /// pixman that sends `State::resize_output` to [`Backend::new`]. The
    /// scanout tier must never get this far -- it resizes by
    /// [`note_resized`](Backend::note_resized) alone, and `resize_output`
    /// answers it before asking this (a debug build asserts as much) --
    /// because a new [`Backend`] would throw its `DrmCompositor`'s renderer
    /// away.
    Unsupported,
    /// The size is over what this renderer can draw into (the answer is its
    /// limit, per axis), so nothing was allocated and nothing changed. No
    /// rebuild can do better: the limit is the device's, and every rebuild
    /// is pinned to the same device.
    TooLarge((i32, i32)),
    /// The pipeline could not reallocate at the new size for some other
    /// reason; nothing changed, and the backend still draws into its old
    /// target at its old size.
    Failed(Box<dyn Error>),
}

/// The renderers [`Backend`] can be carrying.
///
/// Each variant owns a renderer and the target that renderer draws into, and
/// nothing else: everything renderer-agnostic lives on [`Backend`], so a
/// third variant would be a third arm at the five dispatch sites here
/// ([`Backend::new`], [`Backend::capture`], [`Backend::import_dmabuf`],
/// [`Backend::cleanup_texture_cache`], [`draw_frame`]) and no change at all
/// to any element source, which is what the seam is for.
///
/// # Why the GLES variant is boxed and the pixman one is not
///
/// `GlesRenderer` is ~6.4KB by value -- it carries GL's whole function-pointer
/// table inline -- against `PixmanBackend`'s 72 bytes. An unboxed variant
/// would make *every* session's `Pipeline` 6.4KB, including a pixman one that
/// never touches GL, and `State::render` `take`s the whole `Backend` out of
/// `State` and puts it back on **every frame** (see `headless.rs`'s
/// `render()`, which does that so it can hold `&mut State` and `&mut` the
/// renderer at once). That is a 6.4KB memcpy twice a frame, on the default
/// path, bought with nothing. Boxed, it is one startup allocation for the
/// sessions that ask for GLES and a pointer for everyone else.
enum Pipeline {
    /// CPU compositing with pixman: the default, and the only mode that works
    /// with no GPU at all.
    Pixman(PixmanBackend),
    /// GLES compositing into an offscreen renderbuffer, read back to main
    /// memory exactly as pixman's image is. Opt-in, `--headless`/`--nested`
    /// only -- see [`gles`] and [`resolve`].
    Gles(Box<GlesBackend>),
    /// GLES compositing straight into the buffer the CRTC scans out, with no
    /// read-back at all. `--tty --renderer gles`, in a build carrying the
    /// `gpu-scanout` feature -- see [`scanout`] and `tty/scanout.rs`. Boxed
    /// for the same reason the offscreen variant is.
    #[cfg(feature = "gpu-scanout")]
    Scanout(Box<ScanoutBackend>),
}

impl Backend {
    /// Builds the render target for `output` at a given size: a renderer, a
    /// target to draw into, and the damage tracker that pairs with them.
    ///
    /// Shared by `headless::init_named` and `State::resize_output` so the two
    /// can't drift apart -- which is also why `renderer` is a parameter here
    /// and a field on `State`: a resize that cannot happen in place
    /// ([`Backend::resize_in_place`]) rebuilds the pipeline, and it has to
    /// rebuild the one the session was started with.
    /// `scanout` is the renderer `tty::init` already built for the GPU
    /// scanout tier (see [`ScanoutHandoff`]); when it carries one, it *is*
    /// the pipeline and `renderer` is not consulted. Empty on every other
    /// path, including `State::resize_output`, which never reaches here on
    /// that tier (see its own early return).
    ///
    /// `gles_device` pins a GLES build to the device the session's GLES
    /// renderer is already on ([`State::gles_device`]); `None` only for the
    /// session's first build, and ignored by every other renderer. See
    /// `gles::GlesDevice` for why a GLES session may never change device.
    pub(super) fn new(
        output: &Output,
        width: i32,
        height: i32,
        renderer: RendererKind,
        scanout: ScanoutHandoff,
        gles_device: Option<GlesDevice>,
    ) -> Result<Self, Box<dyn Error>> {
        #[cfg(feature = "gpu-scanout")]
        if let Some(backend) = scanout.backend {
            return Ok(Self {
                pipeline: Pipeline::Scanout(backend),
                damage: OutputDamageTracker::from_output(output),
                size: (width, height),
                cursor: CursorInFrame::default(),
                patch_pixels: Vec::new(),
            });
        }
        #[cfg(not(feature = "gpu-scanout"))]
        let _ = scanout;
        let pipeline = match renderer {
            RendererKind::Pixman => Pipeline::Pixman(PixmanBackend::new(width, height)?),
            RendererKind::Gles => {
                Pipeline::Gles(Box::new(GlesBackend::new(width, height, gles_device)?))
            }
        };
        Ok(Self {
            pipeline,
            damage: OutputDamageTracker::from_output(output),
            size: (width, height),
            cursor: CursorInFrame::default(),
            patch_pixels: Vec::new(),
        })
    }

    /// The EGL device this backend's offscreen GLES renderer is on, or `None`
    /// for any other pipeline -- what [`State::gles_device`] reads to pin a
    /// rebuild. The scanout tier answers `None` because it is never rebuilt
    /// through [`Backend::new`] (see `State::resize_output`).
    pub(super) fn gles_device(&self) -> Option<GlesDevice> {
        match &self.pipeline {
            Pipeline::Gles(gpu) => Some(gpu.device),
            Pipeline::Pixman(_) => None,
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(_) => None,
        }
    }

    /// Whether this session composites straight into its scanout buffer.
    ///
    /// The one thing outside this module that has to know: `resize_output`
    /// must *not* rebuild a scanout pipeline, because `DrmCompositor` follows
    /// the output's mode on its own and rebuilding would throw away the EGL
    /// context, the swapchain and the frame in flight to get an identical
    /// one. See that function.
    pub(super) fn is_scanout(&self) -> bool {
        #[cfg(feature = "gpu-scanout")]
        {
            matches!(self.pipeline, Pipeline::Scanout(_))
        }
        #[cfg(not(feature = "gpu-scanout"))]
        {
            false
        }
    }

    /// Moves the recorded render-target size without touching the pipeline --
    /// what a scanout resize is, in full. See [`is_scanout`](Self::is_scanout).
    pub(super) fn note_resized(&mut self, width: i32, height: i32) {
        self.size = (width, height);
    }

    /// Resizes the render target to `width` x `height` on the renderer this
    /// backend already has, where the pipeline does that (see [`InPlace`]).
    ///
    /// The offscreen GLES pipeline does: a whole new backend is a new EGL
    /// context and shader set, 3.95 ms per size on the dev VM's llvmpipe
    /// against 15.7 µs for the renderbuffer alone (`headless::bench`'s
    /// `resize_cost`, LTO off; see `gles::GlesBackend::resize`). pixman does
    /// not, and needs not: its whole backend rebuilds in 11.3 µs there.
    /// A size over the GLES context's limit is refused before anything is
    /// allocated ([`InPlace::TooLarge`]). The scanout tier never reaches
    /// here (`State::resize_output` answers it with
    /// [`note_resized`](Self::note_resized)), and a debug build asserts so.
    ///
    /// On [`InPlace::Resized`] everything size-bound on this backend follows
    /// the new target: the recorded size (what capture clients are told to
    /// match); a fresh damage tracker, since the new target holds nothing
    /// the old one's history describes (headless and nested pass age 0, so
    /// this is a full redraw either way, and the tracker is the same one
    /// [`Backend::new`] would build); and an empty cursor record, since no
    /// frame has drawn a cursor into it. The capture path's region pools
    /// (`capture_cursor::PatchPool`, `patch_pixels`) are keyed on the
    /// region, never the output's size, and stay. On anything else nothing
    /// at all has changed.
    pub(super) fn resize_in_place(&mut self, output: &Output, width: i32, height: i32) -> InPlace {
        debug_assert!(
            !self.is_scanout(),
            "the scanout tier is resized by note_resized, never in place or rebuilt"
        );
        match &mut self.pipeline {
            Pipeline::Gles(gpu) => {
                if let Some(max) = gpu.exceeds_max_target(width, height) {
                    return InPlace::TooLarge(max);
                }
                if let Err(error) = gpu.resize(width, height) {
                    return InPlace::Failed(error);
                }
            }
            Pipeline::Pixman(_) => return InPlace::Unsupported,
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(_) => return InPlace::Unsupported,
        }
        self.damage = OutputDamageTracker::from_output(output);
        self.size = (width, height);
        self.cursor = CursorInFrame::default();
        InPlace::Resized
    }

    /// Which renderer is *actually* drawing this session's frames.
    ///
    /// Deliberately not the same question as `State::renderer`: that field
    /// is what was asked for, this is what was built. A test that runs the
    /// pixel suites under `--renderer gles` has to be able to tell the
    /// difference, or a silent fallback to pixman would make it pass while
    /// proving nothing (see `test_support`); and `dmabuf/renderer_copies.rs`
    /// counts the backends a dma-buf import really went into as GLES, once
    /// per import. Nothing on the frame path branches on either -- that is
    /// [`draw_frame`]'s single match.
    pub(super) fn renderer(&self) -> RendererKind {
        match &self.pipeline {
            Pipeline::Pixman(_) => RendererKind::Pixman,
            Pipeline::Gles(_) => RendererKind::Gles,
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(_) => RendererKind::Gles,
        }
    }

    /// The renderer context of an offscreen GLES backend, `None` for any
    /// other pipeline: the identity a client texture is cached against, so
    /// a suite can pin that a resize kept it (and so re-imported nothing).
    #[cfg(test)]
    pub(super) fn gles_context_for_test(
        &self,
    ) -> Option<smithay::backend::renderer::ContextId<smithay::backend::renderer::gles::GlesTexture>>
    {
        match &self.pipeline {
            Pipeline::Gles(gpu) => Some(gpu.renderer.context_id()),
            _ => None,
        }
    }

    /// An offscreen GLES backend's `GL_MAX_RENDERBUFFER_SIZE`, asked of the
    /// driver afresh, `None` for any other pipeline: one past it is a size
    /// the driver refuses on every device, which is how a suite reaches a
    /// failed reallocation, and what the cached limit is checked against.
    #[cfg(test)]
    pub(super) fn gles_max_renderbuffer_size_for_test(&mut self) -> Option<i32> {
        use smithay::backend::renderer::gles::ffi;
        match &mut self.pipeline {
            Pipeline::Gles(gpu) => gpu
                .renderer
                .with_context(|gl| {
                    let mut max = 0;
                    // SAFETY: a plain state query into a local, on the
                    // context `with_context` has just made current.
                    unsafe { gl.GetIntegerv(ffi::MAX_RENDERBUFFER_SIZE, &mut max) };
                    max
                })
                .ok(),
            _ => None,
        }
    }

    /// The size limit an offscreen GLES backend checks a resize against
    /// (see `gles::GlesBackend::exceeds_max_target`), `None` for any other
    /// pipeline or a driver that would not report one.
    #[cfg(test)]
    pub(super) fn gles_max_target_for_test(&self) -> Option<(i32, i32)> {
        match &self.pipeline {
            Pipeline::Gles(gpu) => gpu.max_target,
            _ => None,
        }
    }

    /// The GL objects this backend's GLES context holds right now, `None`
    /// for pixman. See [`LiveGlObjects`].
    #[cfg(test)]
    pub(crate) fn gles_live_objects_for_test(&mut self) -> Option<LiveGlObjects> {
        match &mut self.pipeline {
            Pipeline::Gles(gpu) => Some(LiveGlObjects::of(&mut gpu.renderer)),
            Pipeline::Pixman(_) => None,
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(gpu) => Some(LiveGlObjects::of(&mut gpu.renderer)),
        }
    }

    /// The render target's size in physical pixels.
    pub(super) fn size(&self) -> (i32, i32) {
        self.size
    }

    /// Reads the whole framebuffer back and hands the bytes to `use_pixels`.
    ///
    /// The two capture consumers -- `screenshot.rs`'s IPC PNG and
    /// `screencopy.rs`'s `ext-image-copy-capture-v1` -- plus the test
    /// harnesses' pixel read-back. Deliberately *not* what the frame path
    /// uses: that one already holds a bound framebuffer and a concrete
    /// renderer, and reads back only the damaged region (see
    /// [`draw_frame_with`]).
    ///
    /// Callback rather than a returned `Vec`, because the pixels are a
    /// borrowed view into the renderer's own mapping: `screencopy` writes
    /// them straight into each due client buffer with no intermediate copy,
    /// and only `screenshot` (which sends them to another thread) actually
    /// owns them.
    pub(super) fn capture<U>(
        &mut self,
        use_pixels: impl FnOnce(&[u8]) -> U,
    ) -> Result<U, CaptureError> {
        let region: Rectangle<i32, Buffer> = Rectangle::from_size(self.size.into());
        match &mut self.pipeline {
            Pipeline::Pixman(cpu) => {
                capture_with(&mut cpu.renderer, &mut cpu.image, region, use_pixels)
            }
            // Both GLES arms free what the capture queued before returning,
            // success or not (see `gles::release_captured`): a capture of a
            // screen that is not redrawing reaches no other drain, and
            // leaked a whole frame per capture without this.
            Pipeline::Gles(gpu) => {
                let read = capture_with(&mut gpu.renderer, &mut gpu.buffer, region, use_pixels);
                gles::release_captured(&mut gpu.renderer);
                read
            }
            // There is no persistent framebuffer to read here: each frame
            // lands in whichever swapchain slot was free, so what a capture
            // binds is the dma-buf that carried the most recent one (see
            // `scanout.rs`). Before the first frame there is nothing to read
            // and saying so is better than handing back an uninitialised
            // buffer.
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(gpu) => {
                let scanout::ScanoutBackend {
                    renderer, captures, ..
                } = &mut **gpu;
                // The decision -- a marked recording refuses loudly rather
                // than serve the pre-direct composite, an empty one refuses
                // rather than bind nothing -- is `Captures::capture_target`'s,
                // pinned there. Both refusals are transient (the next
                // composite frame clears them) and both capture callers
                // force that frame first (`State::ensure_scanout_capture_current`),
                // so reaching one means the force could not draw. `Bind` is
                // the honest stage: there is no current buffer to bind. No
                // new stage, so the presenter-copy path below -- which can
                // never produce this -- keeps its exhaustive three-arm match.
                let frame = captures
                    .capture_target()
                    .map_err(|refusal| CaptureError::new(CaptureStage::Bind, refusal))?;
                // The same two objects as above: binding the slot's dma-buf
                // makes a framebuffer object per bind as well (the texture
                // arm of `Bind<Dmabuf>` at the pinned rev), besides the
                // pixel-pack buffer.
                let read = capture_with(renderer, frame, region, use_pixels);
                gles::release_captured(renderer);
                read
            }
        }
    }

    /// Whether a capture served off this backend right now would read a
    /// stale-or-missing buffer.
    ///
    /// Scanout-tier only: its captures read the last recorded swapchain
    /// slot, which is missing before the first frame and stale after any
    /// primary-direct one (see `scanout::Captures`). Every other tier reads
    /// a persistent framebuffer that is current by construction, so this is
    /// false there. What [`State::ensure_scanout_capture_current`] keys its
    /// forced composite frame on.
    #[cfg(feature = "gpu-scanout")]
    pub(super) fn scanout_capture_stale(&self) -> bool {
        match &self.pipeline {
            Pipeline::Pixman(_) | Pipeline::Gles(_) => false,
            Pipeline::Scanout(gpu) => gpu.captures.capture_stale(),
        }
    }

    /// Why a capture served off this backend right now would be stale, or
    /// `None` when it would read current pixels (always, off the scanout
    /// tier). See [`scanout::Captures::staleness`].
    #[cfg(feature = "gpu-scanout")]
    pub(super) fn scanout_capture_staleness(&self) -> Option<scanout::Stale> {
        match &self.pipeline {
            Pipeline::Pixman(_) | Pipeline::Gles(_) => None,
            Pipeline::Scanout(gpu) => gpu.captures.staleness(),
        }
    }

    /// Imports a client's dma-buf into the renderer, answering whether it
    /// could be.
    ///
    /// The texture is dropped by the caller on purpose: what this
    /// establishes is that the mapping can be made at all, before the client
    /// is told its buffer exists. See `dmabuf.rs`.
    ///
    /// The renderer's error is stringified rather than propagated, because
    /// each renderer has its own error type and the only consumer logs it.
    /// Refusal path only -- a successful import allocates nothing here.
    pub(super) fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> Result<(), String> {
        match &mut self.pipeline {
            Pipeline::Pixman(cpu) => ImportDma::import_dmabuf(&mut cpu.renderer, dmabuf, None)
                .map(|_texture| ())
                .map_err(|error| error.to_string()),
            Pipeline::Gles(gpu) => ImportDma::import_dmabuf(&mut gpu.renderer, dmabuf, None)
                .map(|_texture| ())
                .map_err(|error| error.to_string()),
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(gpu) => ImportDma::import_dmabuf(&mut gpu.renderer, dmabuf, None)
                .map(|_texture| ())
                .map_err(|error| error.to_string()),
        }
    }

    /// Whether this session's renderer can really import a dma-buf of
    /// `format`.
    ///
    /// The question `zwp_linux_dmabuf_v1`'s feedback tranche is built from:
    /// a format advertised here and then refused at import kills the client
    /// that believed it, because `create_immed`'s only failure reply is a
    /// fatal protocol error (see `dmabuf.rs`). So the answer has to come from
    /// the renderer that will actually do the importing -- this one -- rather
    /// than from a list, from `State::renderer` (what was *asked* for, not
    /// what was built) or from a probe of some other EGL display.
    ///
    /// The advertisement asks this only of a renderer whose
    /// [`dmabuf_import_set`](Self::dmabuf_import_set) is
    /// [`ImportSet::CpuMapped`] -- pixman, for each of its two candidates --
    /// and the tests ask it of every renderer. Startup-only either way: never
    /// per import and never per frame.
    pub(super) fn imports_dmabuf_format(&self, format: Format) -> bool {
        match &self.pipeline {
            Pipeline::Pixman(cpu) => ImportDma::has_dmabuf_format(&cpu.renderer, format),
            Pipeline::Gles(gpu) => ImportDma::has_dmabuf_format(&gpu.renderer, format),
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(gpu) => ImportDma::has_dmabuf_format(&gpu.renderer, format),
        }
    }

    /// What kind of answer this session's renderer gives about the dma-bufs
    /// it can import -- the input `dmabuf.rs`'s feedback tranche is derived
    /// from.
    ///
    /// Two kinds, because the two renderers answer different questions:
    ///
    /// - pixman **maps the buffer itself**, so what it can import is bounded
    ///   by what a CPU mapping can make sense of -- a single-plane `LINEAR`
    ///   buffer -- and its advertisement is a fixed candidate list it merely
    ///   narrows ([`ImportSet::CpuMapped`]).
    /// - both GLES tiers **hand the buffer to a driver**, and the driver has
    ///   already said which `{fourcc, modifier}` pairs it takes: the EGL
    ///   display's `dmabuf_texture_formats`, which is what
    ///   `ImportDma::dmabuf_formats` returns for a `GlesRenderer` at the
    ///   pinned rev (`gles/mod.rs:1305`), external-only formats included
    ///   ([`ImportSet::Driver`]).
    ///
    /// Deliberately not [`maps_dmabufs_on_the_cpu`](Self::maps_dmabufs_on_the_cpu),
    /// although today the two split the renderers the same way: that one
    /// answers "does scoot have to synchronise a mapping it reads itself",
    /// this one "what may a client be told to allocate". A renderer that
    /// mapped buffers *and* had a driver's answer would split them.
    ///
    /// Startup-only: `dmabuf.rs::advertise` calls it once per session. The
    /// clone is of a set the EGL display built once at creation.
    pub(super) fn dmabuf_import_set(&self) -> ImportSet {
        match &self.pipeline {
            Pipeline::Pixman(_) => ImportSet::CpuMapped,
            Pipeline::Gles(gpu) => ImportSet::Driver(ImportDma::dmabuf_formats(&gpu.renderer)),
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(gpu) => ImportSet::Driver(ImportDma::dmabuf_formats(&gpu.renderer)),
        }
    }

    /// Whether an imported dma-buf becomes a **CPU mapping this compositor
    /// reads itself**, rather than something the driver samples.
    ///
    /// True for pixman alone, which `mmap`s plane 0 and composites out of that
    /// mapping -- so nothing but scoot synchronises it, and `dmabuf.rs`'s
    /// `sync_committed_dmabufs` has to issue the `DMA_BUF_IOCTL_SYNC` bracket
    /// itself on every commit. Both GLES tiers hand the buffer to the driver
    /// as an `EGLImage` instead and never map it here, so the buffer's
    /// implicit fences are the driver's to honour when it samples -- the
    /// arrangement every GL compositor relies on, none of which issues a
    /// per-commit `DMA_BUF_IOCTL_SYNC`. Running it there would not be free
    /// either: the `START` half *blocks the event loop* until the client's GPU
    /// job finishes, while holding that surface's user-data locks (see
    /// `sync_committed_dmabufs`), which is a cost with no CPU mapping left to
    /// justify it.
    ///
    /// Deliberately *not* folded into
    /// [`State::imports_dmabufs`](super::State), which answers a different
    /// question ("has any import succeeded in this session") and has a second
    /// reader that must stay renderer-agnostic -- the cache drain, which every
    /// renderer needs.
    pub(super) fn maps_dmabufs_on_the_cpu(&self) -> bool {
        match &self.pipeline {
            Pipeline::Pixman(_) => true,
            Pipeline::Gles(_) => false,
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(_) => false,
        }
    }

    /// What this backend's renderer can render *into* as a dma-buf, `None`
    /// for any pipeline `--nested` cannot hand to its host that way: only
    /// the offscreen GLES one. pixman has no GPU buffer to hand over, and the
    /// scanout tier is `--tty`'s alone.
    ///
    /// Startup-only (`nested/gpu.rs`'s `negotiate`): a clone of a set the EGL
    /// display built once.
    #[cfg(feature = "gpu-scanout")]
    pub(super) fn dmabuf_render_formats(&self) -> Option<FormatSet> {
        match &self.pipeline {
            Pipeline::Gles(gpu) => Bind::<Dmabuf>::supported_formats(&gpu.renderer),
            Pipeline::Pixman(_) | Pipeline::Scanout(_) => None,
        }
    }

    /// Copies the frame in this backend's render target into `dmabuf`, on
    /// the GPU (see `gles::copy_into`). Offscreen GLES only; any other
    /// pipeline answers `Err` without touching anything.
    ///
    /// `--nested`'s startup probe, and the suites; the frame path reaches
    /// `gles::copy_into` directly from [`draw_frame`], which already holds
    /// the renderer.
    #[cfg(feature = "gpu-scanout")]
    pub(super) fn copy_frame_into(&mut self, dmabuf: &mut Dmabuf) -> Result<(), Box<dyn Error>> {
        match &mut self.pipeline {
            Pipeline::Gles(gpu) => {
                gles::copy_into(&mut gpu.renderer, &mut gpu.buffer, self.size, dmabuf)
            }
            Pipeline::Pixman(_) | Pipeline::Scanout(_) => {
                Err("only the offscreen GLES renderer can copy a frame into a dma-buf".into())
            }
        }
    }

    /// Whether `width` x `height` is over what this backend's renderer can
    /// draw into, answering the limit if it is -- the same refusal
    /// [`resize_in_place`](Self::resize_in_place) gives
    /// ([`InPlace::TooLarge`]), asked *before* anything is allocated for the
    /// size. `--nested` asks it before building host buffers at a size the
    /// render target would then refuse. `None` for every other pipeline:
    /// pixman has no such limit, and the scanout tier is never resized here.
    pub(super) fn exceeds_max_target(&self, width: i32, height: i32) -> Option<(i32, i32)> {
        match &self.pipeline {
            Pipeline::Gles(gpu) => gpu.exceeds_max_target(width, height),
            Pipeline::Pixman(_) => None,
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(_) => None,
        }
    }

    /// The DRM render node this session's renderer is on, where it has one.
    ///
    /// `None` for pixman, which has no device at all: it `mmap`s whatever
    /// dma-buf it is handed, whichever node allocated it. Both GLES tiers
    /// answer with their own device's render node, which is what a client
    /// has to allocate on for the import to have a chance of succeeding --
    /// see `dmabuf.rs`'s `main_device`.
    ///
    /// The scanout tier has a second rung because its EGL display often
    /// cannot answer at all: it is made through `PLATFORM_GBM_KHR`, whose
    /// `EGLDevice` need not carry `EGL_EXT_device_drm`, so the GBM device it
    /// was made on is asked instead (see [`scanout::ScanoutBackend::node`]).
    /// The two name the same device by construction, so this is a fallback,
    /// not a preference.
    pub(super) fn render_node(&self) -> Option<libc::dev_t> {
        match &self.pipeline {
            Pipeline::Pixman(_) => None,
            Pipeline::Gles(gpu) => gles::render_node(&gpu.renderer),
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(gpu) => gles::render_node(&gpu.renderer).or(gpu.node),
        }
    }

    /// Drops every texture mapping whose buffer has gone.
    ///
    /// See `dmabuf.rs`'s `drain_cache`, the only caller, for why this is
    /// driven from `wl_buffer` destruction rather than from rendering.
    pub(super) fn cleanup_texture_cache(&mut self) -> Result<(), String> {
        match &mut self.pipeline {
            Pipeline::Pixman(cpu) => Renderer::cleanup_texture_cache(&mut cpu.renderer)
                .map_err(|error| error.to_string()),
            Pipeline::Gles(gpu) => Renderer::cleanup_texture_cache(&mut gpu.renderer)
                .map_err(|error| error.to_string()),
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(gpu) => Renderer::cleanup_texture_cache(&mut gpu.renderer)
                .map_err(|error| error.to_string()),
        }
    }
}

impl State {
    /// The EGL device this session's offscreen GLES backends are on, if it
    /// has built one -- the pin every later [`Backend::new`] is handed.
    ///
    /// Read off whichever backend answers, because it is an invariant of all
    /// of them: the first was built unpinned and every other one pinned to
    /// it. A scan of a map of at most `MAX_OUTPUTS` entries, on a resize or
    /// an output being added, never per frame.
    pub(super) fn gles_device(&self) -> Option<GlesDevice> {
        self.backends.values().find_map(Backend::gles_device)
    }
}

#[cfg(feature = "gpu-scanout")]
impl State {
    /// Forces one fully-composited scanout frame when the capture recording
    /// for `id` is stale-or-missing, so the capture served right after reads
    /// current pixels.
    ///
    /// Both capture callers funnel through here before reading: IPC
    /// `screenshot` (`capture_pixels_for`) and `ext-image-copy-capture-v1`
    /// (`service_captures`) -- and through the latter, shell thumbnails and
    /// workspace overviews, which are ext-capture clients and have no other
    /// pixel path. A fresh composite recording short-circuits before
    /// touching the presenter (one map lookup); otherwise: arm the
    /// composite-only frame and render it immediately, synchronously on the
    /// event-loop thread like every other capture-adjacent render. Whether
    /// the swapchain is reset first depends on why the recording is stale
    /// ([`force_needs_reset`]): a recording that is merely behind a direct
    /// frame is refreshed by the plain composite frame, because Smithay
    /// damages the whole output coming back from direct scanout; an empty
    /// recording gets the reset, which forces the full damage a static
    /// screen would otherwise draw nothing on. If the plain frame still
    /// records nothing, the reset path runs after it, so a capture is never
    /// left refused on a screen that simply stopped moving.
    ///
    /// Costs one composite frame (plus, for an empty recording or that
    /// fallback, a swapchain reallocation; and one re-export of a direct
    /// client's framebuffers on its next frame: the forced frame never asks
    /// Smithay to consider the element for a plane, so the element's
    /// framebuffer cache is not carried into that frame's state and lapses),
    /// and only when the recording is actually stale -- direct frames keep
    /// flipping direct between captures. A capture *stream* does not come
    /// through here stale at all: `render::primary_direct` keeps a streamed
    /// output composited (`Screencopy::streaming`), because paying this per
    /// frame measured worse than compositing. While paused (no DRM master) the render
    /// draws nothing and the recording stays stale; the capture then fails
    /// loudly at [`Backend::capture`] rather than serving the old screen.
    /// Never arms without rendering in the same call: a bare arming would
    /// spend its composite frame on unrelated damage and leave the capture
    /// stale anyway. `request_render` also re-arms the frame timer; the
    /// resulting tick finds nothing to do and drops itself, same as after
    /// any other damage-driven frame.
    pub(super) fn ensure_scanout_capture_current(&mut self, id: OutputId) {
        let Some(stale) = self
            .backends
            .get(&id)
            .and_then(Backend::scanout_capture_staleness)
        else {
            return;
        };
        // debug!, not louder: this is the expected path for every capture
        // after a direct frame, and it is once per capture, never per frame
        // -- the line that shows the force path firing in a live session.
        tracing::debug!(
            output = id.0,
            ?stale,
            "capture forces a composite frame: the recording is stale"
        );
        let reset = force_needs_reset(stale);
        self.force_scanout_composite(reset);
        if reset
            || !self
                .backends
                .get(&id)
                .is_some_and(Backend::scanout_capture_stale)
        {
            return;
        }
        // The cheap frame did not record: nothing on screen changed *and*
        // the primary was not direct any more (a direct frame whose commit
        // was refused leaves the recording marked while the plane still
        // shows the old composite), so Smithay had nothing to draw. The
        // reset forces the full redraw that always records -- without it a
        // static screen would refuse every capture until something moved.
        tracing::debug!(
            output = id.0,
            "the forced composite drew nothing; resetting the swapchain and forcing again"
        );
        self.force_scanout_composite(true);
    }

    /// Arms one composite-only frame, optionally resets the swapchain (full
    /// damage), and renders it now. See [`State::ensure_scanout_capture_current`].
    fn force_scanout_composite(&mut self, reset: bool) {
        if let Some(presenter) = self.tty.as_mut().and_then(Tty::scanout_mut) {
            presenter.arm_force_composite();
            if reset {
                presenter.invalidate_scanout();
            }
        }
        self.request_render();
        self.render();
    }
}

/// Whether the composite frame a capture forces must first reset the
/// swapchain, by why the recording is stale.
///
/// [`Stale::Direct`](scanout::Stale::Direct): no. The forced frame composites
/// with both primary bits off, and when the primary goes from a client
/// buffer back to a swapchain slot Smithay treats the whole output as damaged
/// and flips that slot even if the renderer found nothing new to draw
/// (`render_frame`'s `had_direct_scan_out` arm at the pinned rev; the frame
/// is not `is_empty` because the primary plane is not skipped). The slot's
/// content is right because the damage tracker and the swapchain ages both
/// count composite frames only -- a direct frame neither renders nor
/// `submitted()`s a slot -- so the diff it draws is against what that slot
/// really holds. Resetting instead freed every swapchain buffer per capture,
/// which the next frame then reallocated and re-registered with KMS.
///
/// [`Stale::Empty`](scanout::Stale::Empty): yes. There is no recording to
/// diff against, and a static screen would otherwise draw nothing at all.
///
/// A cheap frame that still records nothing falls back to the reset (see
/// the caller), so this is an optimisation that cannot leave a capture
/// refused.
#[cfg(feature = "gpu-scanout")]
fn force_needs_reset(stale: scanout::Stale) -> bool {
    match stale {
        scanout::Stale::Empty => true,
        scanout::Stale::Direct => false,
    }
}

/// [`Backend::capture`]'s body, over any renderer that can bind its own
/// target and read it back -- the same "one generic function, one arm per
/// renderer" shape [`draw_frame`] uses, for the same reason: the arms pick a
/// renderer, nothing below them knows which one.
fn capture_with<R, T, U>(
    renderer: &mut R,
    target: &mut T,
    region: Rectangle<i32, Buffer>,
    use_pixels: impl FnOnce(&[u8]) -> U,
) -> Result<U, CaptureError>
where
    R: Bind<T> + ExportMem,
{
    let framebuffer = renderer
        .bind(target)
        .map_err(|error| CaptureError::new(CaptureStage::Bind, error))?;
    read_back(renderer, &framebuffer, region, use_pixels)
}

/// Which step of a read-back failed. Each caller has its own log line per
/// step, so the step is reported rather than folded into the message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CaptureStage {
    /// The renderer could not bind its own render target.
    Bind,
    /// The framebuffer could not be copied into a mapping.
    Copy,
    /// The mapping could not be mapped into main memory.
    Map,
}

/// A failed read-back: which step, and what the renderer said.
///
/// `Display` is the renderer's own message alone -- `screenshot.rs` reports
/// it verbatim to the IPC client, and the step is already named by the
/// surrounding text there.
#[derive(Debug)]
pub(super) struct CaptureError {
    pub(super) stage: CaptureStage,
    error: String,
}

impl CaptureError {
    fn new(stage: CaptureStage, error: impl fmt::Display) -> Self {
        Self {
            stage,
            error: error.to_string(),
        }
    }
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error)
    }
}

/// Copies `region` out of an already-bound framebuffer and hands the bytes to
/// `use_pixels`.
///
/// `Argb8888` is the same little-endian BGRA layout `wl_shm`'s own
/// `Argb8888` uses (see `screenshot.rs`'s comment on the same fact for the
/// PNG path), so a presenter writing into a `Argb8888`/`Xrgb8888` scanout
/// buffer does a straight memcpy with no channel reordering.
///
/// # This must not branch on `TextureMapping::flipped()`
///
/// It would be an easy-looking "fix" and it would silently invert the screen.
/// Measured with an orientation marker (green at the logical top-left, red at
/// the bottom-left) against both `PixmanRenderer` and `GlesRenderer` on the
/// dev VM: **the two produce byte-identical buffer layouts -- buffer row 0 is
/// the top for both -- while `PixmanMapping::flipped()` answers `false` and
/// `GlesMapping::flipped()` answers `true`.** The GLES flag describes GL's
/// own bottom-left origin, which Smithay has *already* compensated for by
/// composing `flip180` into the projection (`gles/mod.rs:2289` at the pinned
/// rev); honouring it here would undo that compensation a second time.
///
/// So the contract this function offers is "the bytes as the renderer laid
/// them out, top row first", and every consumer -- the presenters' memcpy,
/// the damage rect they write it at, the PNG encode, every capture client --
/// depends on it uniformly.
fn read_back<R, U>(
    renderer: &mut R,
    framebuffer: &R::Framebuffer<'_>,
    region: Rectangle<i32, Buffer>,
    use_pixels: impl FnOnce(&[u8]) -> U,
) -> Result<U, CaptureError>
where
    R: ExportMem,
{
    // Written out rather than chained because both intermediates are
    // borrowed from: the pixel slice's lifetime comes from the *mapping*
    // (see `map_texture`'s signature at the pinned rev), not from `&mut
    // self`, so the mapping has to outlive the callback. Nothing here copies
    // the frame a second time.
    let mapping = renderer
        .copy_framebuffer(framebuffer, region, Fourcc::Argb8888)
        .map_err(|error| CaptureError::new(CaptureStage::Copy, error))?;
    let pixels = renderer
        .map_texture(&mapping)
        .map_err(|error| CaptureError::new(CaptureStage::Map, error))?;
    Ok(use_pixels(pixels))
}

/// What one frame did, for the lifecycle around it (`State::render`'s tail)
/// to act on.
///
/// Every field is "nothing happened" by default, which is exactly what a
/// failed bind or a failed draw leaves behind: the tail then confirms no
/// lock, stamps no presentation feedback and wakes no cursor client, which
/// is the required behaviour for a frame that never reached the screen.
#[derive(Default)]
pub(super) struct FrameOutcome {
    /// Whether this frame actually reached the renderer. Only a frame that
    /// did may confirm a pending session lock: the protocol forbids sending
    /// `locked` before a blanked frame exists (see `session_lock.rs`).
    pub(super) drew_a_frame: bool,
    /// The flip this frame went out on under `--tty`, if `present` issued
    /// one. Only meaningful for a locked frame with a pending lock (see the
    /// `confirm_lock`/`await_vblank` split in `State::render`); every other
    /// frame leaves it for the tail to ignore.
    pub(super) blank_seq: Option<u64>,
    /// Whether this frame's `present` was refused after the pixels were
    /// already written (a commit/page-flip the kernel rejected -- the only
    /// present skip no completion event can retry, since nothing is in
    /// flight). The tail re-arms the frame timer for it.
    pub(super) retry_render: bool,
    /// Whether this frame reached the host under `--nested`, if `present`
    /// committed it. Like `blank_seq`, this is what tells the tail the frame
    /// actually went out rather than merely rendered: only a presented frame
    /// may stamp presentation feedback (see `presentation_time.rs`).
    pub(super) host_committed: bool,
    /// The client-supplied cursor surface this frame drew from, if any --
    /// set only on the frames that actually went looking for one (`--tty`
    /// with a pointer). See `State::render`'s `send_frames_surface_tree`
    /// call.
    pub(super) cursor_surface: Option<WlSurface>,
    /// The element whose client buffer this frame scanned out directly on
    /// the primary plane, if it did: its surface's presentation feedback
    /// carries `zero_copy` (see `presentation_time.rs`). Only the GPU
    /// scanout tier can set it; every other tier copies every buffer.
    pub(super) zero_copy: Option<Id>,
    /// Whether this frame changed the pixels on screen: it drew, and the
    /// damage tracker (or `DrmCompositor`) reported a change. What
    /// `State::render` counts into `State::frame_serial` -- for a frame
    /// that something other than the cursor asked for.
    pub(super) damaged: bool,
}

/// The colour a frame is cleared to: the lock screen's while locked, the
/// configured background otherwise. One function for every tier -- and for
/// `render::primary_direct`, whose rule 6 must judge the very colour
/// `DrmCompositor::render_frame` is handed.
fn frame_clear_color(state: &State, locked: bool) -> Color32F {
    if locked {
        state.lock_clear_color()
    } else {
        state.appearance.background_color.into()
    }
}

/// Draws one frame with whichever renderer `backend` is carrying, and hands
/// it to whichever presenters are watching.
///
/// The one place a concrete renderer is chosen. Everything downstream of the
/// match arm is generic, so a second [`Pipeline`] variant is a second arm
/// here and nothing else.
pub(super) fn draw_frame(
    state: &mut State,
    backend: &mut Backend,
    output: &Output,
    locked: bool,
) -> FrameOutcome {
    #[cfg(test)]
    if std::mem::take(&mut state.fail_next_draw_for_test) {
        return FrameOutcome::default();
    }
    let Backend {
        pipeline,
        damage,
        size,
        cursor,
        ..
    } = backend;
    match pipeline {
        Pipeline::Pixman(cpu) => {
            let PixmanBackend {
                renderer, image, ..
            } = cpu;
            // pixman has no GPU buffer to hand over: the host always gets a
            // read-back from this arm.
            draw_frame_with(
                state, renderer, image, damage, cursor, *size, output, locked, false,
            )
            .0
        }
        Pipeline::Gles(gpu) => {
            let GlesBackend {
                renderer, buffer, ..
            } = &mut **gpu;
            // Read once, here: the same answer decides whether the generic
            // body reads the frame back and whether this arm blits it.
            let by_dmabuf = state
                .host
                .as_ref()
                .is_some_and(super::nested::Host::presents_dmabuf);
            // The target is about to be drawn over: a frame owed from it is
            // no longer there to hand over (see `Host::begin_frame`).
            #[cfg(feature = "gpu-scanout")]
            if by_dmabuf && let Some(host) = &mut state.host {
                host.begin_frame();
            }
            #[cfg_attr(not(feature = "gpu-scanout"), allow(unused_mut))]
            let (mut outcome, owed) = draw_frame_with(
                state, renderer, buffer, damage, cursor, *size, output, locked, by_dmabuf,
            );
            // The frame the host is owed, as a dma-buf rather than bytes:
            // copied on the GPU into a free host buffer and committed (see
            // `nested/gpu.rs`). `owed` is only ever `Some` when `by_dmabuf`
            // was, and without the feature `presents_dmabuf` is never true.
            #[cfg(feature = "gpu-scanout")]
            if let (Some(region), Some(host)) = (owed, &mut state.host) {
                let presented = host.present_dmabuf(*size, region, |dmabuf| {
                    gles::copy_into(renderer, buffer, *size, dmabuf)
                });
                outcome.host_committed = presented.committed;
                // A path that just fell back to read-back owes the host a
                // frame it has not had: ask for one, once.
                outcome.retry_render |= presented.fell_back;
            }
            #[cfg(not(feature = "gpu-scanout"))]
            let _ = owed;
            outcome
        }
        // A separate body, not a third `draw_frame_with` arm: this tier has
        // no read-back, no presenter hand-off and no use for `damage` at all
        // (`DrmCompositor` owns its own `OutputDamageTracker`), so sharing
        // one function would mean branching inside it on which half of it
        // applies.
        #[cfg(feature = "gpu-scanout")]
        Pipeline::Scanout(gpu) => draw_frame_scanout(state, gpu, *size, output, locked),
    }
}

/// [`draw_frame`]'s body for the GPU scanout tier.
///
/// The shape the other tiers have -- bind a framebuffer, render into it, read
/// it back, hand the bytes to a presenter -- collapses to one call here:
/// `DrmCompositor::render_frame` binds the swapchain slot itself and
/// `queue_frame` puts it on the CRTC. What is left to do around it is exactly
/// what the other body does *outside* its own render call: build the frame
/// context, gather the elements, pick the clear colour, and report what
/// happened.
///
/// `damage` is deliberately not a parameter. `Backend`'s own
/// `OutputDamageTracker` is superseded on this tier, not merely unused: the
/// compositor tracks damage against its own swapchain's buffer ages, which is
/// the only tracking that can be right when the buffer a frame lands in is
/// chosen by the swapchain.
#[cfg(feature = "gpu-scanout")]
fn draw_frame_scanout(
    state: &mut State,
    gpu: &mut ScanoutBackend,
    size: (i32, i32),
    output: &Output,
    locked: bool,
) -> FrameOutcome {
    let mut outcome = FrameOutcome::default();
    let scanout::ScanoutBackend {
        renderer,
        captures,
        last_eligibility,
        judge_scratch,
        ..
    } = gpu;
    let clear_color = frame_clear_color(state, locked);
    let (elements, cursor_surface, direct) = scanout_frame_elements(
        state,
        renderer,
        size,
        output,
        locked,
        clear_color,
        judge_scratch,
    );
    outcome.cursor_surface = cursor_surface;
    let output_id = state.outputs.id_of(output);
    if direct != *last_eligibility {
        // debug!, and only on a change: this is the line that says a
        // session started or stopped going direct, and why -- once per
        // transition, never per frame.
        tracing::debug!(
            from = ?*last_eligibility,
            to = ?direct,
            "scanout: primary-direct eligibility changed"
        );
        *last_eligibility = direct;
    }

    let (drawn, retry) = {
        // Unreachable in practice -- this pipeline only exists on a `--tty`
        // session -- but written as a fallthrough rather than an `expect`,
        // because a panic on the frame path would take every client's
        // unsaved state with it. The default `FrameOutcome` says "nothing
        // happened", which confirms no lock and stamps no feedback.
        let Some(presenter) = state.tty.as_mut().and_then(Tty::scanout_mut) else {
            tracing::warn!("the scanout pipeline has no drm compositor to present to");
            return outcome;
        };
        // Before the render, not after: the swapchain may have been rebuilt
        // since the last frame (a mode change, a reactivation), which frees
        // every slot the capture pool exported a dma-buf from.
        if presenter.take_slots_dropped() {
            captures.forget_slots();
        }
        // The scanout tranche for this plane set, rebuilt only when the key
        // moves (startup, a CRTC switch, a modifier the exporter newly
        // refused) -- one comparison on every other frame. Here because the
        // presenter is what knows the plane; steering is below, once the
        // frame is out.
        if let Some(id) = output_id {
            state.scanout_feedback.refresh(
                id,
                presenter.scanout_formats_key(),
                state.dmabuf_default.as_ref(),
                || presenter.scanout_formats(),
            );
        }
        let drawn = presenter.render_and_queue(
            renderer,
            &elements,
            clear_color,
            direct.allowed(),
            (output.current_scale().fractional_scale(), size),
            state.drm_syncobj.explicit(),
            |buffer, cursor| {
                captures.note_frame(buffer, cursor);
            },
        );
        // The direct arm's half of the capture contract: the slot the
        // recording points at was never drawn into by this frame, so mark
        // it rather than leaving a stale composite readable as current. A
        // capture served off the mark forces a composite frame first (see
        // `ensure_scanout_capture_current`).
        if drawn.primary_direct.is_some() {
            captures.note_direct();
        }
        (drawn, presenter.take_retry_render())
    };

    // Per-surface dma-buf feedback: the covering window is steered toward a
    // layout the primary plane can take while this output is eligible, and
    // back once it is not (see `dmabuf/scanout.rs`). Sends only on a change.
    if let Some(id) = output_id {
        state.steer_scanout_feedback(id, direct.allowed(), Instant::now);
    }

    outcome.drew_a_frame = drawn.drew;
    outcome.blank_seq = drawn.flip;
    outcome.retry_render = retry;
    outcome.zero_copy = drawn.primary_direct;
    // Exactly what `draw_frame_with` reports: whether the pixels moved (see
    // `FrameOutcome::damaged`).
    outcome.damaged = drawn.damaged;
    outcome
}

/// The scanout tier's frame list, and whether that frame may go
/// primary-direct: what [`draw_frame_scanout`] composites and flags.
///
/// Generic over the renderer, although the tier only ever runs it with its
/// `GlesRenderer`, so the harness can drive the very code the tier runs
/// with whichever renderer a headless `State` carries (see
/// [`State::primary_direct_now`]) -- the eligibility is a function of the
/// gathered list, and gathering is renderer-agnostic.
///
/// The eligibility is judged from the list just gathered and the `locked`
/// it was gathered with, so the flags and the elements describe the same
/// frame. No window and no ring is laid out while locked (the same rule as
/// `draw_frame_with`), because no frame can show them.
#[cfg(feature = "gpu-scanout")]
fn scanout_frame_elements<R>(
    state: &mut State,
    renderer: &mut R,
    size: (i32, i32),
    output: &Output,
    locked: bool,
    clear_color: Color32F,
    judge_scratch: &mut primary_direct::JudgeScratch,
) -> (
    Vec<Elements<R>>,
    Option<WlSurface>,
    primary_direct::PrimaryDirect,
)
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Send + Clone + 'static,
{
    let frame = FrameContext {
        size,
        scale: output.current_scale().fractional_scale(),
        geometry: state.space.output_geometry(output),
        output: state.outputs.id_of(output),
        locked,
    };
    let arrangement = if locked {
        None
    } else {
        Some(state.world.arrange())
    };
    let ring_elements: Vec<Elements<R>> = ring_elements(
        &mut state.decorations,
        &state.appearance,
        &state.windows,
        arrangement.as_ref(),
        &frame,
        renderer,
    );
    let draws_cursor = state.frame_draws_cursor();
    let (elements, cursor_surface) = state.gather_elements(
        renderer,
        output,
        &frame,
        ring_elements,
        arrangement.as_ref(),
        draws_cursor,
    );
    let tried_with = primary_direct::TriedWith {
        size: frame.size,
        scale: frame.scale,
        clear_color,
    };
    let direct = primary_direct::judge(
        state,
        renderer,
        output,
        locked,
        &elements,
        &tried_with,
        judge_scratch,
    );
    (elements, cursor_surface, direct)
}

/// What [`draw_frame_scanout`] would decide about the primary output's next
/// frame, judged over the list this headless session gathers -- the harness
/// side of [`scanout_frame_elements`], which is the code the tier runs.
#[cfg(all(test, feature = "gpu-scanout"))]
impl State {
    pub(super) fn primary_direct_now(&mut self) -> primary_direct::PrimaryDirect {
        let (id, output) = self
            .outputs
            .at(0)
            .expect("a headless harness has an output");
        let mut backend = self.take_backend(id).expect("a render target");
        let locked = self.session_lock.is_locked();
        let size = backend.size;
        let clear_color = frame_clear_color(self, locked);
        let mut scratch = primary_direct::JudgeScratch::default();
        let direct = match &mut backend.pipeline {
            Pipeline::Pixman(cpu) => {
                scanout_frame_elements(
                    self,
                    &mut cpu.renderer,
                    size,
                    &output,
                    locked,
                    clear_color,
                    &mut scratch,
                )
                .2
            }
            Pipeline::Gles(gpu) => {
                scanout_frame_elements(
                    self,
                    &mut gpu.renderer,
                    size,
                    &output,
                    locked,
                    clear_color,
                    &mut scratch,
                )
                .2
            }
            Pipeline::Scanout(_) => unreachable!("no test builds a scanout pipeline"),
        };
        self.put_backend(id, backend);
        direct
    }
}

/// Times [`primary_direct::judge`] alone over the primary output's current
/// frame: the list is gathered once (as `draw_frame_scanout` would), then
/// judged `rounds` times with one scratch, as the tier judges frame after
/// frame. Answers the verdict and the mean time per call.
#[cfg(all(test, feature = "gpu-scanout"))]
impl State {
    pub(super) fn judge_cost(
        &mut self,
        rounds: u32,
    ) -> (primary_direct::PrimaryDirect, std::time::Duration) {
        let (id, output) = self
            .outputs
            .at(0)
            .expect("a headless harness has an output");
        let mut backend = self.take_backend(id).expect("a render target");
        let locked = self.session_lock.is_locked();
        let size = backend.size;
        let clear_color = frame_clear_color(self, locked);
        let mut scratch = primary_direct::JudgeScratch::default();
        let Pipeline::Pixman(cpu) = &mut backend.pipeline else {
            unreachable!("the cost harness runs on pixman");
        };
        let (elements, _, verdict) = scanout_frame_elements(
            self,
            &mut cpu.renderer,
            size,
            &output,
            locked,
            clear_color,
            &mut scratch,
        );
        let tried_with = primary_direct::TriedWith {
            size,
            scale: output.current_scale().fractional_scale(),
            clear_color,
        };
        let started = std::time::Instant::now();
        for _ in 0..rounds {
            let again = primary_direct::judge(
                self,
                &mut cpu.renderer,
                &output,
                locked,
                &elements,
                &tried_with,
                &mut scratch,
            );
            std::hint::black_box(again);
        }
        let each = started.elapsed() / rounds.max(1);
        drop(elements);
        self.put_backend(id, backend);
        (verdict, each)
    }
}

/// [`draw_frame`]'s body, over any renderer that can import client buffers,
/// bind its own target and read that target back.
///
/// `T` is the renderer's own target type (a `pixman::Image` today); it is a
/// type parameter rather than an associated type because Smithay's [`Bind`]
/// is parameterised by the target, so one renderer may bind several kinds.
///
/// `host_by_dmabuf` says the `--nested` host takes this frame as a dma-buf
/// the caller copies on the GPU rather than as read-back bytes: the frame is
/// then not read back for the host at all, and the second half of the answer
/// is the damage the caller owes it (`None` when nothing drew or nothing
/// changed). Only the GLES arm of [`draw_frame`] ever passes `true`.
#[allow(clippy::too_many_arguments)]
fn draw_frame_with<R, T>(
    state: &mut State,
    renderer: &mut R,
    target: &mut T,
    damage: &mut OutputDamageTracker,
    cursor: &mut CursorInFrame,
    size: (i32, i32),
    output: &Output,
    locked: bool,
    host_by_dmabuf: bool,
) -> (FrameOutcome, Option<Rectangle<i32, Physical>>)
where
    R: Renderer + ImportAll + ImportMem + Bind<T> + ExportMem,
    R::TextureId: Texture + Send + Clone + 'static,
{
    let mut outcome = FrameOutcome::default();
    let mut owed = None;
    let (width, height) = size;
    let frame = FrameContext {
        size,
        scale: output.current_scale().fractional_scale(),
        geometry: state.space.output_geometry(output),
        output: state.outputs.id_of(output),
        locked,
    };
    // The core's arrangement, and this frame's ring segments built from it --
    // computed once here rather than inside the match below so a failure to
    // bind the framebuffer still logs without having done this for nothing.
    // See `decorations.rs`'s module doc for why the background isn't part of
    // this list.
    //
    // Not computed at all while locked: no window and no ring is drawn then,
    // so laying the windows out would be work for a frame that cannot show
    // it. `apply()` still runs the layout on every change underneath, so
    // nothing is lost by the time it unlocks.
    let arrangement = if locked {
        None
    } else {
        Some(state.world.arrange())
    };
    let ring_elements: Vec<Elements<_>> = ring_elements(
        &mut state.decorations,
        &state.appearance,
        &state.windows,
        arrangement.as_ref(),
        &frame,
        renderer,
    );
    match renderer.bind(target) {
        Ok(mut framebuffer) => {
            let draws_cursor = state.frame_draws_cursor();
            let (elements, cursor_surface) = state.gather_elements(
                renderer,
                output,
                &frame,
                ring_elements,
                arrangement.as_ref(),
                draws_cursor,
            );
            outcome.cursor_surface = cursor_surface;

            // `0` (always-full-redraw) for every presenter except `--tty`:
            // see `buffers.rs`'s module doc on why that one specifically
            // needs a real buffer age -- cursor motion can trigger a render
            // on every mouse-motion event, and a naive full-frame copy at
            // that rate is exactly the multi-MB/s memcpy the roadmap calls
            // out. Neither `--headless` nor `--nested` draws a cursor, so
            // neither has a new reason to render more often than before.
            let age = state.tty.as_ref().map_or(0, Tty::next_buffer_age);
            // The backdrop element already covers the output opaquely while
            // locked; this is the second line of defence behind it, so that
            // even a frame whose elements somehow produced nothing clears to
            // the lock colour rather than to the configured desktop
            // background -- which a user may have given an alpha, and which
            // is the colour the unlocked session is showing.
            let clear_color = frame_clear_color(state, locked);
            let result =
                damage.render_output(renderer, &mut framebuffer, age, &elements, clear_color);
            match result {
                Ok(render_result) => {
                    outcome.drew_a_frame = true;
                    // The framebuffer now holds exactly this list, damaged
                    // or not (an undamaged frame is one whose list drew the
                    // same pixels already there), so this is what a capture
                    // read from it finds of the cursor. A failed render
                    // leaves the previous record with the previous pixels.
                    // Skipped outright when the frame drew no cursor -- the
                    // record is then the default, and the list is not
                    // walked.
                    *cursor = if draws_cursor {
                        CursorInFrame::of(
                            &elements,
                            frame.scale.into(),
                            Rectangle::from_size(size.into()),
                            |_| None,
                        )
                    } else {
                        CursorInFrame::default()
                    };
                    // Whether the pixels moved: the damage tracker having
                    // something to report, not reaching this arm at all --
                    // under `--tty` (the one backend passing a real buffer
                    // age) a redundant `request_render` legitimately draws
                    // nothing and leaves the previous frame on screen. See
                    // `FrameOutcome::damaged`.
                    outcome.damaged = render_result.damage.is_some();
                    // Must run unconditionally, even when there turns out to
                    // be nothing to present below -- see
                    // `BufferPool::advance_generation`'s doc for why this
                    // can't be skipped just because this frame is.
                    if let Some(tty) = &mut state.tty {
                        tty.advance_generation();
                    }
                    // Both presenters read back the same frame the same way;
                    // only what happens with the pixels afterward differs, so
                    // the read-back itself happens once for whichever (or
                    // both) are set -- and not at all with neither (plain
                    // `--headless`, e.g. under IPC-only control): that copy
                    // would be pure waste on every render with nothing to
                    // hand it to.
                    if host_by_dmabuf && state.host.is_some() {
                        // The host takes this frame as a dma-buf the caller
                        // copies on the GPU: nothing to read back. `--nested`
                        // has no `--tty` beside it, so no other presenter is
                        // skipped by this. The whole frame is owed (age 0,
                        // see above), reported as the bbox of what the
                        // tracker says changed, exactly as the read-back
                        // below would have.
                        owed = render_result.damage.map(|damaged| union_bbox(damaged));
                    } else if (state.host.is_some() || state.tty.is_some())
                        && let Some(damaged) = render_result.damage
                    {
                        // Bounding box of every damaged rect, not the rects
                        // themselves: the read-back (and the dumb-buffer
                        // write behind it) only ever copies one contiguous
                        // region. For `--headless`/`--nested` (`age` always
                        // `0` above) this is always the full frame
                        // regardless, so nothing changes for them; for
                        // `--tty` a small, cheap bbox is the common case (see
                        // `buffers.rs`'s module doc), and only a large
                        // pointer jump or a real content change grows it.
                        let region = union_bbox(damaged);
                        let buffer_region: Rectangle<i32, Buffer> = Rectangle::new(
                            (region.loc.x, region.loc.y).into(),
                            (region.size.w, region.size.h).into(),
                        );
                        // Under GLES this read-back's pixel-pack buffer is
                        // queued, not deleted, when it drops. It is not drained
                        // here the way a capture's is (`gles::release_captured`):
                        // the next frame's `finish` drains it, so at most one
                        // is ever outstanding, and it only exists because a
                        // frame was drawn.
                        let outcome = &mut outcome;
                        let read = read_back(renderer, &framebuffer, buffer_region, |pixels| {
                            if let Some(host) = &mut state.host {
                                outcome.host_committed =
                                    host.present(pixels, region.size.w, region.size.h);
                            }
                            if let Some(tty) = &mut state.tty {
                                outcome.blank_seq = tty.present(pixels, region, (width, height));
                                outcome.retry_render = tty.take_retry_render();
                            }
                        });
                        if let Err(failure) = read {
                            match failure.stage {
                                // `read_back` cannot report `Bind` -- the
                                // framebuffer is already bound here -- but
                                // the arm is written out rather than
                                // `unreachable!()`, because a panic on the
                                // frame path would take every client's
                                // unsaved state with it.
                                CaptureStage::Bind | CaptureStage::Copy => tracing::warn!(
                                    error = %failure,
                                    "could not copy the framebuffer for the presenter"
                                ),
                                CaptureStage::Map => tracing::warn!(
                                    error = %failure,
                                    "could not read back the frame for the presenter"
                                ),
                            }
                        }
                    }
                    // else: no presenter is watching this frame, or (only
                    // possible when `age > 0`, i.e. only under `--tty`)
                    // nothing actually changed -- e.g. a redundant
                    // `request_render` with no real difference -- so there's
                    // nothing to read back or present either way.
                }
                Err(error) => tracing::warn!(%error, "could not render"),
            }
        }
        Err(error) => tracing::warn!(%error, "could not bind the framebuffer"),
    }
    (outcome, owed)
}

/// The smallest rectangle containing every rect in `rects`. A free function
/// so it's testable without a live renderer, same rationale as `input.rs`'s
/// `clamp_to_extent`. `rects` must be non-empty --
/// `OutputDamageTracker::render_output` only ever returns `Some` damage when
/// it has at least one rectangle to report; an empty list would have been
/// `None` instead (confirmed against the pinned Smithay source).
fn union_bbox(rects: &[Rectangle<i32, Physical>]) -> Rectangle<i32, Physical> {
    let mut iter = rects.iter().copied();
    let first = iter
        .next()
        .expect("render_output never returns an empty damage list");
    iter.fold(first, |acc, rect| {
        let x0 = acc.loc.x.min(rect.loc.x);
        let y0 = acc.loc.y.min(rect.loc.y);
        let x1 = (acc.loc.x + acc.size.w).max(rect.loc.x + rect.size.w);
        let y1 = (acc.loc.y + acc.size.h).max(rect.loc.y + rect.size.h);
        Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into())
    })
}

/// How many names [`LiveGlObjects::of`] probes in each namespace: far above
/// anything one test's context allocates.
///
/// The probe also checks a freshly reserved name against it. That is a
/// sanity check, not a guarantee. It catches a context that hands out names
/// monotonically and has outgrown the range. A context that reuses the
/// lowest free name, as Mesa can, may answer a low name while live objects
/// sit higher. The tests stay far enough below the range (a few dozen names
/// each) that neither case arises.
#[cfg(test)]
const PROBED_GL_NAMES: u32 = 1 << 12;

/// The GL buffer and framebuffer objects alive in a GLES context, counted by
/// name.
///
/// What the capture-path tests measure (`render/tests/capture_release.rs`,
/// `screencopy/tests.rs`): Smithay's `GlesRenderer` defers deleting a GL
/// object whose handle dropped until its cleanup queue is next drained, and
/// an object still in that queue answers `glIsBuffer`/`glIsFramebuffer`
/// true. So this counts the queue itself, deterministically, where process
/// memory would only show the allocator's high-water mark.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LiveGlObjects {
    /// Buffer objects. A read-back's pixel-pack buffer is one, the size of
    /// the region it read -- a whole frame, for a screenshot.
    pub(crate) buffers: u32,
    /// Framebuffer objects. Binding a renderbuffer target makes one.
    pub(crate) framebuffers: u32,
}

#[cfg(test)]
impl LiveGlObjects {
    fn of(renderer: &mut smithay::backend::renderer::gles::GlesRenderer) -> Self {
        renderer
            .with_context(|gl| {
                // SAFETY: name reservations and name queries only, on the
                // context `with_context` has just made current. The two
                // names reserved are never bound, so they create no object,
                // and are released again before the count.
                unsafe {
                    let (mut buffer, mut framebuffer) = (0, 0);
                    gl.GenBuffers(1, &mut buffer);
                    gl.GenFramebuffers(1, &mut framebuffer);
                    gl.DeleteBuffers(1, &buffer);
                    gl.DeleteFramebuffers(1, &framebuffer);
                    assert!(
                        buffer < PROBED_GL_NAMES && framebuffer < PROBED_GL_NAMES,
                        "the context hands out names past the probe \
                         ({buffer}, {framebuffer}); raise PROBED_GL_NAMES"
                    );
                    let mut live = Self {
                        buffers: 0,
                        framebuffers: 0,
                    };
                    for name in 1..PROBED_GL_NAMES {
                        live.buffers += u32::from(gl.IsBuffer(name) != 0);
                        live.framebuffers += u32::from(gl.IsFramebuffer(name) != 0);
                    }
                    live
                }
            })
            .expect("the GLES context can be made current")
    }
}

#[cfg(test)]
mod tests;
