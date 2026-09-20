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

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Format, Fourcc};
use smithay::backend::renderer::damage::OutputDamageTracker;
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
use gles::GlesBackend;
use pixman::PixmanBackend;

mod elements;
mod gles;
mod pixman;
#[cfg(feature = "gpu-scanout")]
mod scanout;

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
/// Built once by `headless::init_named` and rebuilt by
/// `State::resize_output`; lives in `State::backend` and is `take`n for the
/// duration of a frame so the render path can hold `&mut State` and `&mut`
/// the renderer at the same time.
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
    /// and a field on `State`: a resize rebuilds the pipeline, and it has to
    /// rebuild the one the session was started with.
    /// `scanout` is the renderer `tty::init` already built for the GPU
    /// scanout tier (see [`ScanoutHandoff`]); when it carries one, it *is*
    /// the pipeline and `renderer` is not consulted. Empty on every other
    /// path, including `State::resize_output`, which never reaches here on
    /// that tier (see its own early return).
    pub(super) fn new(
        output: &Output,
        width: i32,
        height: i32,
        renderer: RendererKind,
        scanout: ScanoutHandoff,
    ) -> Result<Self, Box<dyn Error>> {
        #[cfg(feature = "gpu-scanout")]
        if let Some(backend) = scanout.backend {
            return Ok(Self {
                pipeline: Pipeline::Scanout(backend),
                damage: OutputDamageTracker::from_output(output),
                size: (width, height),
            });
        }
        #[cfg(not(feature = "gpu-scanout"))]
        let _ = scanout;
        let pipeline = match renderer {
            RendererKind::Pixman => Pipeline::Pixman(PixmanBackend::new(width, height)?),
            RendererKind::Gles => Pipeline::Gles(Box::new(GlesBackend::new(width, height)?)),
        };
        Ok(Self {
            pipeline,
            damage: OutputDamageTracker::from_output(output),
            size: (width, height),
        })
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

    /// Which renderer is *actually* drawing this session's frames.
    ///
    /// Test-only, and deliberately not the same question as
    /// `State::renderer`: that field is what was asked for, this is what was
    /// built. A test that runs the pixel suites under `--renderer gles` has
    /// to be able to tell the difference, or a silent fallback to pixman
    /// would make it pass while proving nothing (see `test_support`).
    /// Nothing on the frame path branches on either -- that is
    /// [`draw_frame`]'s single match.
    #[cfg(test)]
    pub(super) fn renderer(&self) -> RendererKind {
        match &self.pipeline {
            Pipeline::Pixman(_) => RendererKind::Pixman,
            Pipeline::Gles(_) => RendererKind::Gles,
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(_) => RendererKind::Gles,
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
            Pipeline::Gles(gpu) => {
                capture_with(&mut gpu.renderer, &mut gpu.buffer, region, use_pixels)
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
                let Some(frame) = captures.frame_mut() else {
                    return Err(CaptureError::new(
                        CaptureStage::Bind,
                        "nothing has been scanned out yet",
                    ));
                };
                capture_with(renderer, frame, region, use_pixels)
            }
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
    /// Startup-only: `dmabuf.rs::advertise` calls it once per candidate
    /// format, never per import and never per frame.
    pub(super) fn imports_dmabuf_format(&self, format: Format) -> bool {
        match &self.pipeline {
            Pipeline::Pixman(cpu) => ImportDma::has_dmabuf_format(&cpu.renderer, format),
            Pipeline::Gles(gpu) => ImportDma::has_dmabuf_format(&gpu.renderer, format),
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(gpu) => ImportDma::has_dmabuf_format(&gpu.renderer, format),
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
    let Backend {
        pipeline,
        damage,
        size,
    } = backend;
    match pipeline {
        Pipeline::Pixman(cpu) => {
            let PixmanBackend { renderer, image } = cpu;
            draw_frame_with(state, renderer, image, damage, *size, output, locked)
        }
        Pipeline::Gles(gpu) => {
            let GlesBackend { renderer, buffer } = &mut **gpu;
            draw_frame_with(state, renderer, buffer, damage, *size, output, locked)
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
    let frame = FrameContext {
        size,
        scale: output.current_scale().fractional_scale(),
        geometry: state.space.output_geometry(output),
        locked,
    };
    let scanout::ScanoutBackend {
        renderer, captures, ..
    } = gpu;
    // Same rule as `draw_frame_with`: no window and no ring is laid out while
    // locked, because no frame can show it.
    let arrangement = if locked {
        None
    } else {
        Some(state.world.arrange())
    };
    let ring_elements: Vec<Elements<_>> = ring_elements(
        &mut state.decorations,
        &state.appearance,
        arrangement.as_ref(),
        &frame,
        renderer,
    );
    let clear_color: Color32F = if locked {
        state.lock_clear_color()
    } else {
        state.appearance.background_color.into()
    };
    let (elements, cursor_surface) = state.gather_elements(
        renderer,
        output,
        &frame,
        ring_elements,
        arrangement.as_ref(),
    );
    outcome.cursor_surface = cursor_surface;

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
        let drawn = presenter.render_and_queue(renderer, &elements, clear_color, |buffer| {
            captures.note_frame(buffer);
        });
        (drawn, presenter.take_retry_render())
    };

    outcome.drew_a_frame = drawn.drew;
    outcome.blank_seq = drawn.flip;
    outcome.retry_render = retry;
    // Exactly what `draw_frame_with` counts: whether the pixels moved. A
    // render that produced no damage left the previous frame on screen, and
    // counting it would make every capture session copy the same pixels
    // again (see `State::frame_serial`).
    if drawn.damaged {
        state.frame_serial = state.frame_serial.wrapping_add(1);
    }
    outcome
}

/// [`draw_frame`]'s body, over any renderer that can import client buffers,
/// bind its own target and read that target back.
///
/// `T` is the renderer's own target type (a `pixman::Image` today); it is a
/// type parameter rather than an associated type because Smithay's [`Bind`]
/// is parameterised by the target, so one renderer may bind several kinds.
fn draw_frame_with<R, T>(
    state: &mut State,
    renderer: &mut R,
    target: &mut T,
    damage: &mut OutputDamageTracker,
    size: (i32, i32),
    output: &Output,
    locked: bool,
) -> FrameOutcome
where
    R: Renderer + ImportAll + ImportMem + Bind<T> + ExportMem,
    R::TextureId: Texture + Send + Clone + 'static,
{
    let mut outcome = FrameOutcome::default();
    let (width, height) = size;
    let frame = FrameContext {
        size,
        scale: output.current_scale().fractional_scale(),
        geometry: state.space.output_geometry(output),
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
        arrangement.as_ref(),
        &frame,
        renderer,
    );
    match renderer.bind(target) {
        Ok(mut framebuffer) => {
            let (elements, cursor_surface) = state.gather_elements(
                renderer,
                output,
                &frame,
                ring_elements,
                arrangement.as_ref(),
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
            let clear_color: Color32F = if locked {
                state.lock_clear_color()
            } else {
                state.appearance.background_color.into()
            };
            let result =
                damage.render_output(renderer, &mut framebuffer, age, &elements, clear_color);
            match result {
                Ok(render_result) => {
                    outcome.drew_a_frame = true;
                    // What `screencopy.rs` asks "have the pixels moved since
                    // this session's last capture?" with. Gated on the damage
                    // tracker having something to report rather than on
                    // reaching this arm at all: under `--tty` (the one
                    // backend passing a real buffer age) a redundant
                    // `request_render` legitimately draws nothing and leaves
                    // the previous frame on screen, and counting that as a
                    // change would make every capture session copy the same
                    // pixels again. See `State::frame_serial`'s doc for what
                    // this does and does not claim.
                    if render_result.damage.is_some() {
                        state.frame_serial = state.frame_serial.wrapping_add(1);
                    }
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
                    if (state.host.is_some() || state.tty.is_some())
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
    outcome
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

#[cfg(test)]
mod tests;
