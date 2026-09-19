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
//! # What a second implementation has to bring, and what it inherits
//!
//! Damage tracking and the framebuffer's size are renderer-agnostic and live
//! on [`Backend`] itself, so a second implementation brings only a renderer
//! and a target (see [`pixman::PixmanBackend`]) and inherits the rest. The
//! three things it has to satisfy are the bounds on [`draw_frame_with`]:
//! import client buffers ([`ImportAll`] + [`ImportMem`]), bind its own target
//! ([`Bind`]), and read that target back to main memory ([`ExportMem`]).
//!
//! # The read-back's orientation, and the trap in it
//!
//! [`read_back`] hands out the framebuffer's bytes exactly as the renderer
//! laid them out, and **deliberately does not consult
//! `TextureMapping::flipped()`** -- see its own doc for why that is a
//! correctness requirement and not an oversight.

use std::error::Error;
use std::fmt;

use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::{
    Bind, Color32F, ExportMem, ImportAll, ImportDma, ImportMem, Renderer, Texture,
};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Buffer, Physical, Rectangle};

use super::State;
use super::tty::Tty;
use elements::FrameContext;
use pixman::PixmanBackend;

mod elements;
mod pixman;

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
/// One variant today. A GPU renderer is a second variant plus a second arm
/// in [`draw_frame`] and [`Backend::capture`] -- not a change to any element
/// source, which is what the seam is for.
enum Pipeline {
    /// CPU compositing with pixman: the default, and the only mode that works
    /// with no GPU at all.
    Pixman(PixmanBackend),
}

impl Backend {
    /// Builds the render target for `output` at a given size: a renderer, a
    /// target to draw into, and the damage tracker that pairs with them.
    ///
    /// Shared by `headless::init_named` and `State::resize_output` so the two
    /// can't drift apart.
    pub(super) fn new(output: &Output, width: i32, height: i32) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            pipeline: Pipeline::Pixman(PixmanBackend::new(width, height)?),
            damage: OutputDamageTracker::from_output(output),
            size: (width, height),
        })
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
                let PixmanBackend { renderer, image } = cpu;
                let framebuffer = renderer
                    .bind(image)
                    .map_err(|error| CaptureError::new(CaptureStage::Bind, error))?;
                read_back(renderer, &framebuffer, region, use_pixels)
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
        }
    }
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
    }
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
    let ring_elements = if locked {
        Vec::new()
    } else {
        let arrangement = state.world.arrange();
        state
            .decorations
            .elements(&arrangement, &state.appearance, frame.bounds(), frame.scale)
    };
    match renderer.bind(target) {
        Ok(mut framebuffer) => {
            let (elements, cursor_surface) =
                state.gather_elements(renderer, output, &frame, ring_elements);
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
