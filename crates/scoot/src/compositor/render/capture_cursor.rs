//! The pointer in a capture, as the capture asks for it rather than as the
//! tier happened to draw it.
//!
//! A capture reads back the frame the compositor last drew for an output,
//! and whether that frame holds the cursor has always been a property of the
//! tier, not of the request:
//!
//! - `--headless` and `--nested` never draw a cursor on screen (see
//!   `cursor.rs`), so their frames never hold one;
//! - `--tty` on the dumb tier composites the cursor into every frame, so its
//!   frames always hold one;
//! - `--tty` on the GPU scanout tier puts the cursor on a KMS plane wherever
//!   the display takes it, so its frames hold it only where the plane
//!   refused.
//!
//! An agent reading a screenshot needs the answer not to change with the
//! renderer, and `ext-image-copy-capture-v1` says outright that the cursor
//! is composited into a capture exactly when the session asked
//! (`paint_cursors`). So every capture now carries the cursor the *request*
//! asked for, on every tier: IPC `screenshot` by its `cursor` field
//! (default: included), `ext-image-copy-capture-v1` by `paint_cursors`.
//!
//! # How: re-render the cursor's region, never blend onto the copy
//!
//! When the recorded frame already matches the request, nothing happens and
//! nothing is gathered beyond the cursor's own elements. When it does not,
//! the region the mismatch covers -- where the cursor sits now (when it is
//! asked for), where the recorded frame composited it, and where a cursor on
//! an overlay plane may have left a hole, clamped to the output -- is
//! rendered again from the frame's own element list, with the cursor or
//! without it, into an offscreen target of that region's size, by the same
//! renderer that draws the output's frames. The result replaces that region
//! of the captured copy.
//!
//! Replaced, not blended over, and that is what makes it correct in every
//! case rather than the common one:
//!
//! - **"Out when not asked"** is only possible this way. A cursor
//!   composited into the frame cannot be subtracted back out of it; what was
//!   under it has to be drawn again.
//! - **Never two cursors.** Blending the cursor onto a copy that already
//!   held one (a frame whose plane refused the cursor, or a composite forced
//!   by a capture -- `State::ensure_scanout_capture_current`) would double
//!   it. Replacing the union of both places cannot: the region ends up
//!   holding exactly what the request asked for, whatever it held before.
//! - **An underlay leaves no hole.** Where a cursor rides an overlay plane
//!   *below* the primary, Smithay punches a transparent hole in the primary
//!   over it, and the swapchain slot a capture reads holds that hole. Every
//!   overlay-planed cursor footprint is recorded
//!   ([`CursorInFrame::on_overlay`]) and always in the region, whether the
//!   pointer was asked for or not -- so plain `grim` and `--no-cursor` get
//!   the scene there, a pointer-requesting capture gets the cursor, and a
//!   pointer that moved since the frame leaves no hole behind. Not seen on
//!   hardware: virtio has no overlay plane; pinned with a synthetic record.
//! - **Same pixels the screen shows.** The cursor comes from the same
//!   `Cursor::element` call the frame makes -- image, hotspot, scale -- and
//!   the region is drawn by the same renderer from the same element list,
//!   moved by a whole number of pixels. Relocation changes no sampling, so
//!   the region matches what a full composite would have drawn there.
//!
//! It is also bounded: the region is the cursor's size (tens of pixels
//! square), not the output's. The worst case is a client cursor surface as
//! large as the output, which costs one full-output render per capture --
//! the ceiling the whole-frame alternative would have paid every time. And
//! a capture *stream* that has its region re-rendered every frame reuses
//! one target, one damage tracker, its element lists and its pixel buffer
//! ([`PatchPool`], [`Backend::recycle_patch`]); what still allocates per
//! capture is listed there.
//!
//! # What "the recorded frame holds the cursor" is read from
//!
//! Not guessed from the tier. Each frame records a [`CursorInFrame`] beside
//! the pixels a capture reads: where the cursor elements it composited
//! landed, where any rode an overlay plane (a possible underlay hole), and
//! whether any reached the screen some other way (a KMS plane). The dumb tier and the offscreen renderers record it from the list
//! they drew (`render::draw_frame_with`); the scanout tier records it from
//! the `DrmCompositor`'s own answer about which elements it put on which
//! plane (`tty::scanout::ScanoutPresenter::render_and_queue`), alongside
//! the swapchain slot it records for captures, so the two cannot describe
//! different frames.

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::utils::{Relocate, RelocateRenderElement};
use smithay::backend::renderer::element::{Element, Id, Kind, render_elements};
use smithay::backend::renderer::gles::{GlesRenderbuffer, GlesRenderer};
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::{
    Bind, Color32F, ImportAll, ImportMem, Offscreen, Renderer, Texture,
};
use smithay::output::Output;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Transform};

use scoot_core::OutputId;

use super::elements::{Elements, FrameContext, ring_elements};
use super::gles::GlesBackend;
use super::{Backend, Pipeline, frame_clear_color, read_back};
use crate::compositor::State;
use crate::compositor::cursor::CursorElement;
use crate::compositor::rounded::Rounded;

#[cfg(test)]
mod tests;

/// What the frame a capture reads holds of the cursor.
///
/// Recorded per frame by whatever records the pixels a capture reads, so the
/// two always describe the same frame (see this module's doc).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CursorInFrame {
    /// The bounding box of the cursor elements composited into the frame,
    /// in output-physical pixels and clamped to the output. `None` when the
    /// frame composited no cursor pixel at all.
    pub(crate) composited: Option<Rectangle<i32, Physical>>,
    /// The bounding box of the cursor elements that rode an **overlay**
    /// plane, clamped like `composited`. The frame may hold a transparent
    /// hole there, not merely lack the cursor: an overlay whose zpos is below
    /// the primary's is an *underlay*, and for every element Smithay puts on
    /// one it draws a hole-punch element into the primary instead
    /// (`DrmCompositor::render_frame`'s `is_underlay` arm, `drm/compositor/
    /// mod.rs` ~2224 at the pinned rev) -- and `Kind::Cursor` elements are
    /// overlay candidates (`try_assign_overlay_plane`, ~3733). Which
    /// overlays are underlays is not asked: every overlay-planed footprint
    /// is treated as a possible hole, and a capture re-renders it *whatever
    /// it asked for* (see [`patch_region`]).
    ///
    /// The cursor plane is deliberately not recorded here. Only the overlay
    /// arm punches holes, so a slot under a cursor-plane cursor holds the
    /// scene as composited, which is already right for a capture that did
    /// not ask for the pointer.
    pub(crate) on_overlay: Option<Rectangle<i32, Physical>>,
    /// Whether some cursor element of the frame reached the screen without
    /// being composited into it -- on the KMS cursor plane or an overlay
    /// plane. Such a frame's pixels lack (part of) the cursor.
    pub(crate) off_frame: bool,
}

/// Which kind of plane took an element out of the frame, as far as the
/// capture cares: only an overlay can leave a hole (see
/// [`CursorInFrame::on_overlay`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Plane {
    Cursor,
    Overlay,
}

impl CursorInFrame {
    /// The record for a frame drawn from `elements`, where `on_plane` says
    /// which elements reached the screen other than through the frame, and
    /// on which kind of plane.
    ///
    /// A cursor element is one of kind [`Kind::Cursor`]: `Cursor::element`
    /// builds every one of them that way (both this compositor's shapes and a
    /// client's cursor surface tree), and nothing else in the tree uses the
    /// kind. Each geometry is clamped to `bounds` *before* the union, which
    /// keeps the arithmetic inside the output: a client-sized cursor surface
    /// (a viewport can make one billions of pixels wide) can never overflow
    /// the union, and an element entirely off this output adds nothing.
    ///
    /// One pass over the list, allocation-free: it runs once per drawn
    /// frame.
    pub(crate) fn of<E: Element>(
        elements: &[E],
        scale: Scale<f64>,
        bounds: Rectangle<i32, Physical>,
        on_plane: impl Fn(&Id) -> Option<Plane>,
    ) -> Self {
        let mut record = Self::default();
        for element in elements {
            if element.kind() != Kind::Cursor {
                continue;
            }
            let plane = on_plane(element.id());
            if plane.is_some() {
                record.off_frame = true;
            }
            if plane == Some(Plane::Cursor) {
                continue;
            }
            let Some(visible) = clamp(element.geometry(scale), bounds) else {
                continue;
            };
            if plane == Some(Plane::Overlay) {
                record.on_overlay = union(record.on_overlay, Some(visible));
            } else {
                record.composited = union(record.composited, Some(visible));
            }
        }
        record
    }
}

/// `rect` inside `bounds`, or `None` where nothing of it is (including a
/// zero-area overlap, which `Rectangle::intersection` reports for rects that
/// merely touch).
fn clamp(
    rect: Rectangle<i32, Physical>,
    bounds: Rectangle<i32, Physical>,
) -> Option<Rectangle<i32, Physical>> {
    rect.intersection(bounds)
        .filter(|inside| inside.size.w > 0 && inside.size.h > 0)
}

/// The smallest rectangle holding both, either of which may be absent.
/// Callers only ever pass rects already clamped to one output, so the
/// corner arithmetic cannot overflow.
fn union(
    a: Option<Rectangle<i32, Physical>>,
    b: Option<Rectangle<i32, Physical>>,
) -> Option<Rectangle<i32, Physical>> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.merge(b)),
        (a, None) => a,
        (None, b) => b,
    }
}

/// The region of a capture that has to be rendered again so it shows the
/// cursor exactly as `want` asks, or `None` when the recorded frame already
/// does.
///
/// `recorded` is what the frame the capture reads holds of the cursor;
/// `now` is where the cursor's elements sit at capture time (already
/// clamped to the output, `None` when there is nothing to draw: hidden, no
/// pointer, a client cursor surface with no content yet, or a pointer on
/// another output).
///
/// Whatever was asked, the frame's cursor pixels (`composited`) and its
/// possible underlay holes (`on_overlay`) are never right to keep where
/// they no longer belong, so both are always in the region when present:
///
/// - **Asked for.** Nothing to do only when the frame composited the whole
///   cursor exactly where it is now and nothing rode a plane. Anything else
///   -- a plane took some of it, the frame never drew it (headless,
///   nested), or it moved since -- re-renders the frame's cursor pixels, its
///   holes and where the cursor is now, so the old places lose their stale
///   pixels (or their hole) and the new one gains the cursor.
/// - **Not asked for.** The frame's cursor pixels and its holes, and
///   nothing else: where a cursor-plane cursor sits the slot already holds
///   the scene without it.
///
/// Pure, so every combination is pinned without a renderer.
pub(super) fn patch_region(
    recorded: CursorInFrame,
    now: Option<Rectangle<i32, Physical>>,
    want: bool,
) -> Option<Rectangle<i32, Physical>> {
    let stale = union(recorded.composited, recorded.on_overlay);
    if !want {
        return stale;
    }
    if recorded.composited == now && !recorded.off_frame {
        return None;
    }
    union(stale, now)
}

/// One re-rendered region of a capture: tightly packed rows of
/// little-endian BGRA (`Argb8888`, the framebuffer's own layout), `rect.w *
/// 4` bytes each, to be written over the captured copy at `rect`.
pub(crate) struct CursorPatch {
    pub(crate) rect: Rectangle<i32, Physical>,
    pub(crate) pixels: Vec<u8>,
}

impl CursorPatch {
    /// Row `y` of the patch (`0..rect.size.h`), or `None` out of range.
    pub(crate) fn row(&self, y: i32) -> Option<&[u8]> {
        if y < 0 || y >= self.rect.size.h {
            return None;
        }
        let row = self.rect.size.w as usize * 4;
        self.pixels.get(y as usize * row..)?.get(..row)
    }

    /// Writes the patch over a tightly packed `width` x `height` BGRA frame
    /// -- the IPC screenshot's own copy.
    ///
    /// Checked rather than trusted, although the patch is clamped to the
    /// same output this frame was read from: a patch that would reach past
    /// the frame is dropped whole (the capture keeps the frame's own
    /// pixels, and says so) rather than written in part or indexed out of
    /// range.
    pub(crate) fn apply(&self, frame: &mut [u8], width: i32, height: i32) {
        if !self.fits(width, height) || frame.len() < width as usize * height as usize * 4 {
            tracing::warn!(
                rect = ?self.rect,
                width,
                height,
                "a capture's cursor region does not fit the frame; leaving the frame as read"
            );
            return;
        }
        let stride = width as usize * 4;
        let x = self.rect.loc.x as usize * 4;
        for y in 0..self.rect.size.h {
            let Some(src) = self.row(y) else {
                return;
            };
            let at = (self.rect.loc.y + y) as usize * stride + x;
            frame[at..at + src.len()].copy_from_slice(src);
        }
    }

    /// Whether the patch lies inside a `width` x `height` frame and carries
    /// exactly one row of pixels per row of its rect. What both writers
    /// check before turning its numbers into offsets.
    pub(crate) fn fits(&self, width: i32, height: i32) -> bool {
        let Rectangle { loc, size } = self.rect;
        loc.x >= 0
            && loc.y >= 0
            && size.w > 0
            && size.h > 0
            && loc
                .x
                .checked_add(size.w)
                .is_some_and(|right| right <= width)
            && loc
                .y
                .checked_add(size.h)
                .is_some_and(|bottom| bottom <= height)
            && self.pixels.len() == size.w as usize * size.h as usize * 4
    }
}

// The frame's element list, moved by the region's origin. `Rounded` has its
// own arm because it cannot be relocated from outside (see
// `Rounded::relocate`); everything else moves by wrapping.
render_elements! {
    pub(super) PatchElement<R> where R: ImportAll + ImportMem;
    Moved = RelocateRenderElement<Elements<R>>,
    Rounded = Rounded<RelocateRenderElement<WaylandSurfaceRenderElement<R>>>,
}

fn relocate<R>(element: Elements<R>, by: Point<i32, Physical>) -> PatchElement<R>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Send + Clone + 'static,
{
    match element {
        Elements::RoundedSurface(rounded) => PatchElement::Rounded(rounded.relocate(by)),
        other => PatchElement::Moved(RelocateRenderElement::from_element(
            other,
            by,
            Relocate::Relative,
        )),
    }
}

impl Backend {
    /// What the frame a capture of this backend reads holds of the cursor.
    ///
    /// The scanout tier keeps its record beside the swapchain slot it
    /// records (`scanout::Captures`); the others read a persistent
    /// framebuffer, whose record lives here.
    pub(crate) fn cursor_in_frame(&self) -> CursorInFrame {
        match &self.pipeline {
            Pipeline::Pixman(_) | Pipeline::Gles(_) => self.cursor,
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(gpu) => gpu.captures.cursor(),
        }
    }
}

/// What the capture path keeps between captures on one output, so a capture
/// stream that has its cursor region re-rendered on every frame does not
/// allocate for it on every frame.
///
/// Per pipeline, because everything here is typed by the renderer: the
/// element lists hold that renderer's elements (always emptied again before
/// a capture returns, so no texture is kept alive by them), and the target
/// is that renderer's own offscreen buffer. The target and its damage
/// tracker are kept at the size and scale of the last region and replaced
/// only when those change -- in a stream the region is the cursor's
/// footprint every time, so that is a cursor-size or output-scale change,
/// or the pointer reaching an output edge. The region's pixels are pooled
/// on [`Backend`] instead (see [`Backend::recycle_patch`]), since they
/// outlive the renderer borrow.
///
/// What still allocates per capture, stated so it is not mistaken for
/// pooled: the parts of the gather the frame path shares (the arrangement,
/// each source's own element list, a client cursor surface tree's list --
/// the same allocations every drawn frame makes), and under GLES Smithay's
/// own per-call GL objects at the pinned rev (`Bind<GlesRenderbuffer>`
/// creates a framebuffer object per bind; `ExportMem::copy_framebuffer` a
/// pixel-pack buffer per read). The pixman path reads its target's bits
/// directly and allocates nothing of its own.
pub(super) struct PatchPool<R, T>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: 'static,
{
    /// Boxed: the damage tracker inside is several hundred bytes, and the
    /// pool lives inline in the pipeline `State::render` moves out of and
    /// back into `State` every frame (see `render.rs`'s note on why the GLES
    /// pipeline is boxed). Allocated when built, not per capture.
    target: Option<Box<PooledTarget<T>>>,
    cursor: Vec<CursorElement<R>>,
    gathered: Vec<Elements<R>>,
    moved: Vec<PatchElement<R>>,
    /// How many targets this pool has built: what the suites read to pin
    /// that a stream reuses one rather than building one per frame.
    #[cfg(test)]
    built: u32,
}

impl<R, T> Default for PatchPool<R, T>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: 'static,
{
    fn default() -> Self {
        Self {
            target: None,
            cursor: Vec::new(),
            gathered: Vec::new(),
            moved: Vec::new(),
            #[cfg(test)]
            built: 0,
        }
    }
}

/// An offscreen target, and the damage tracker that renders into it, for
/// one region size at one scale.
struct PooledTarget<T> {
    size: (i32, i32),
    scale: u64,
    target: T,
    damage: OutputDamageTracker,
}

/// An offscreen target the capture path can render a region into and read
/// straight back: one impl per renderer, because the read is not the same
/// shape on each (see [`PatchPool`]).
pub(super) trait PatchTarget<R>: Sized
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: 'static,
{
    /// A target of `size`, in `Argb8888` like the framebuffer.
    fn create(renderer: &mut R, size: (i32, i32)) -> Result<Self, String>;

    /// Renders `elements` over `clear_color` into the whole target (of
    /// `size`) with a full redraw, and writes its pixels as tightly packed
    /// `Argb8888` rows into `out` (cleared first; its capacity is reused).
    fn render_into(
        &mut self,
        renderer: &mut R,
        damage: &mut OutputDamageTracker,
        elements: &[PatchElement<R>],
        clear_color: Color32F,
        size: (i32, i32),
        out: &mut Vec<u8>,
    ) -> Result<(), String>;
}

impl PatchTarget<PixmanRenderer> for pixman::Image<'static, 'static> {
    fn create(renderer: &mut PixmanRenderer, size: (i32, i32)) -> Result<Self, String> {
        renderer
            .create_buffer(Fourcc::Argb8888, size.into())
            .map_err(|error| format!("could not allocate the region: {error}"))
    }

    fn render_into(
        &mut self,
        renderer: &mut PixmanRenderer,
        damage: &mut OutputDamageTracker,
        elements: &[PatchElement<PixmanRenderer>],
        clear_color: Color32F,
        size: (i32, i32),
        out: &mut Vec<u8>,
    ) -> Result<(), String> {
        {
            let mut framebuffer = renderer
                .bind(self)
                .map_err(|error| format!("could not bind the region: {error}"))?;
            damage
                .render_output(renderer, &mut framebuffer, 0, elements, clear_color)
                .map_err(|error| format!("could not render the region: {error:?}"))?;
        }
        // Read the image's own bits rather than through `ExportMem`, whose
        // pixman `copy_framebuffer` allocates a fresh image per call. The
        // render above is synchronous and its framebuffer is dropped, so the
        // bits are complete and nothing else borrows them.
        let (width, height) = (self.width(), self.height());
        let row = width * 4;
        let stride = self.stride();
        if !matches!(self.format(), pixman::FormatCode::A8R8G8B8)
            || stride < row
            || (width, height) != (size.0.max(0) as usize, size.1.max(0) as usize)
        {
            return Err(format!(
                "the region's image is not {width}x{height} Argb8888 (stride {stride})"
            ));
        }
        out.clear();
        out.reserve(row * height);
        // SAFETY: `data()` points at this image's own bits, `stride * height`
        // bytes allocated by pixman when the image was created and alive as
        // long as `self`, which this function holds `&mut` -- the renderer's
        // framebuffer borrow ended with the block above, so no writer
        // aliases them. Each row read is `row <= stride` bytes from a row
        // start below `stride * height`, so every read is inside the
        // allocation.
        let bits = unsafe { self.data() }.cast::<u8>().cast_const();
        for y in 0..height {
            let slice = unsafe { std::slice::from_raw_parts(bits.add(y * stride), row) };
            out.extend_from_slice(slice);
        }
        Ok(())
    }
}

impl PatchTarget<GlesRenderer> for GlesRenderbuffer {
    fn create(renderer: &mut GlesRenderer, size: (i32, i32)) -> Result<Self, String> {
        renderer
            .create_buffer(Fourcc::Argb8888, size.into())
            .map_err(|error| format!("could not allocate the region: {error}"))
    }

    fn render_into(
        &mut self,
        renderer: &mut GlesRenderer,
        damage: &mut OutputDamageTracker,
        elements: &[PatchElement<GlesRenderer>],
        clear_color: Color32F,
        (width, height): (i32, i32),
        out: &mut Vec<u8>,
    ) -> Result<(), String> {
        let rendered = render_and_read(
            self,
            renderer,
            damage,
            elements,
            clear_color,
            (width, height),
            out,
        );
        // Once the region's framebuffer and mapping have dropped, success or
        // not: the region render's own `finish` drained the queue *before*
        // this read-back, so its pixel-pack buffer and the bind's framebuffer
        // object would otherwise wait for the next frame (see
        // `gles::release_captured`).
        super::gles::release_captured(renderer);
        rendered
    }
}

/// [`PatchTarget::render_into`]'s body for [`GlesRenderbuffer`], split out so
/// the caller can drain what it queued on every path out, `?` included.
fn render_and_read(
    target: &mut GlesRenderbuffer,
    renderer: &mut GlesRenderer,
    damage: &mut OutputDamageTracker,
    elements: &[PatchElement<GlesRenderer>],
    clear_color: Color32F,
    (width, height): (i32, i32),
    out: &mut Vec<u8>,
) -> Result<(), String> {
    let mut framebuffer = renderer
        .bind(target)
        .map_err(|error| format!("could not bind the region: {error}"))?;
    damage
        .render_output(renderer, &mut framebuffer, 0, elements, clear_color)
        .map_err(|error| format!("could not render the region: {error:?}"))?;
    let whole: Rectangle<i32, Buffer> = Rectangle::from_size((width, height).into());
    read_back(renderer, &framebuffer, whole, |pixels| {
        pack_rows(pixels, width, height, out)
    })
    .map_err(|error| format!("could not read the region back: {error}"))?
}

impl State {
    /// The region of output `id`'s next capture that has to be re-rendered
    /// for it to show the cursor exactly as `want` asks, rendered -- or
    /// `None` when the frame the capture reads already does (or the region
    /// could not be rendered, which is logged and leaves the capture as
    /// read). Hand the patch back with [`Backend::recycle_patch`] once it is
    /// written, so its pixel buffer is reused.
    ///
    /// `backend` is `id`'s own render target, taken out of `State` by the
    /// caller for the duration of the capture. Nothing here touches the
    /// frame lifecycle: no `needs_render`, no `frame_serial`, no frame
    /// callbacks, and the element sources it gathers from are idempotent
    /// for an unchanged scene (the ring buffers only take a new commit when
    /// their size or colour changes), so the next frame's damage is exactly
    /// what it would have been.
    ///
    /// Cost: when the frame already matches, one cursor-element build (a
    /// texture lookup for this compositor's own shapes) and nothing else.
    /// Otherwise one element gather -- what every frame does -- and one
    /// render of a cursor-sized region, plus that region's read-back, into
    /// buffers kept in [`PatchPool`].
    pub(crate) fn capture_cursor_patch(
        &mut self,
        backend: &mut Backend,
        id: OutputId,
        want: bool,
    ) -> Option<CursorPatch> {
        let recorded = backend.cursor_in_frame();
        // Nothing to take out of a frame that composited no cursor and
        // holds no possible underlay hole: no gather, no renderer.
        if !want && recorded.composited.is_none() && recorded.on_overlay.is_none() {
            return None;
        }
        let output = self.outputs.get(id)?.clone();
        let size = backend.size;
        let mut pixels = backend.patch_pixels.pop().unwrap_or_default();
        let region = match &mut backend.pipeline {
            Pipeline::Pixman(cpu) => render_patch(
                self,
                &mut cpu.renderer,
                &mut cpu.patch,
                size,
                &output,
                recorded,
                want,
                &mut pixels,
            ),
            Pipeline::Gles(gpu) => {
                let GlesBackend {
                    renderer, patch, ..
                } = &mut **gpu;
                render_patch(
                    self,
                    renderer,
                    patch,
                    size,
                    &output,
                    recorded,
                    want,
                    &mut pixels,
                )
            }
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(gpu) => {
                let super::scanout::ScanoutBackend {
                    renderer, patch, ..
                } = &mut **gpu;
                render_patch(
                    self,
                    renderer,
                    patch,
                    size,
                    &output,
                    recorded,
                    want,
                    &mut pixels,
                )
            }
        };
        match region {
            Some(rect) => Some(CursorPatch { rect, pixels }),
            None => {
                backend.recycle_pixels(pixels);
                None
            }
        }
    }
}

#[cfg(test)]
impl Backend {
    /// Replaces what the frame this backend reads is recorded to hold of the
    /// cursor: how the suites stand in for a scanout frame whose cursor rode
    /// an overlay (an underlay hole), which no harness -- and no hardware
    /// this project has -- can produce.
    pub(crate) fn set_cursor_in_frame_for_test(&mut self, record: CursorInFrame) {
        self.cursor = record;
    }

    /// How many region targets this backend's capture pool has built.
    pub(crate) fn patch_targets_built(&self) -> u32 {
        match &self.pipeline {
            Pipeline::Pixman(cpu) => cpu.patch.built,
            Pipeline::Gles(gpu) => gpu.patch.built,
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(gpu) => gpu.patch.built,
        }
    }
}

impl Backend {
    /// Takes a written patch back, keeping its pixel buffer for the next
    /// one. At most two are kept -- one per answer a tick can need (with
    /// the pointer, without it).
    pub(crate) fn recycle_patch(&mut self, patch: CursorPatch) {
        self.recycle_pixels(patch.pixels);
    }

    fn recycle_pixels(&mut self, mut pixels: Vec<u8>) {
        if self.patch_pixels.len() < 2 {
            pixels.clear();
            self.patch_pixels.push(pixels);
        }
    }
}

/// [`State::capture_cursor_patch`]'s body over whichever renderer the
/// backend carries: decides the region, and renders it into `pixels` when
/// there is one, answering the region.
///
/// Gathers exactly as the frame does (same `locked`, same arrangement, same
/// ring). The cursor's own elements are built first, alone -- they decide
/// the region -- and then put in front of the rest of the list when `want`
/// asks for them, rather than built a second time by the gather.
#[allow(clippy::too_many_arguments)]
fn render_patch<R, T>(
    state: &mut State,
    renderer: &mut R,
    pool: &mut PatchPool<R, T>,
    size: (i32, i32),
    output: &Output,
    recorded: CursorInFrame,
    want: bool,
    pixels: &mut Vec<u8>,
) -> Option<Rectangle<i32, Physical>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Send + Clone + 'static,
    T: PatchTarget<R>,
{
    let locked = state.session_lock.is_locked();
    let frame = FrameContext {
        size,
        scale: output.current_scale().fractional_scale(),
        geometry: state.space.output_geometry(output),
        output: state.outputs.id_of(output),
        locked,
    };
    let scale = Scale::from(frame.scale);
    let bounds = Rectangle::from_size(size.into());
    // Where the cursor sits now, from its own elements alone: the common
    // case (a frame that already matches) stops here without gathering the
    // rest of the scene.
    state.cursor_elements_into(renderer, &frame, &mut pool.cursor);
    let now = CursorInFrame::of(&pool.cursor, scale, bounds, |_| None).composited;
    let Some(region) = patch_region(recorded, now, want) else {
        pool.cursor.clear();
        return None;
    };

    let arrangement = if locked {
        None
    } else {
        Some(state.world.arrange())
    };
    let ring = ring_elements(
        &mut state.decorations,
        &state.appearance,
        &state.windows,
        arrangement.as_ref(),
        &frame,
        renderer,
    );
    state.gather_elements_into(
        renderer,
        output,
        &frame,
        ring,
        arrangement.as_ref(),
        false,
        &mut pool.gathered,
    );
    let by = Point::from((-region.loc.x, -region.loc.y));
    // Front to back, as the frame lists it: the cursor first (it is drawn
    // over everything, the lock screen included), then the scene.
    if want {
        pool.moved.extend(
            pool.cursor
                .drain(..)
                .map(|element| relocate(Elements::Cursor(element), by)),
        );
    }
    pool.cursor.clear();
    pool.moved
        .extend(pool.gathered.drain(..).map(|element| relocate(element, by)));
    let clear_color = frame_clear_color(state, locked);

    let region_size = (region.size.w, region.size.h);
    let scale_bits = frame.scale.to_bits();
    let rendered = (|| -> Result<(), String> {
        let reuse = pool
            .target
            .as_ref()
            .is_some_and(|kept| kept.size == region_size && kept.scale == scale_bits);
        if !reuse {
            // Replaced, not kept beside: the old one is dropped first, so at
            // most one target lives per output.
            pool.target = None;
            #[cfg(test)]
            {
                pool.built += 1;
            }
            pool.target = Some(Box::new(PooledTarget {
                size: region_size,
                scale: scale_bits,
                target: T::create(renderer, region_size)?,
                damage: OutputDamageTracker::new(region_size, scale, Transform::Normal),
            }));
        }
        let Some(kept) = pool.target.as_mut() else {
            return Err("the region's target went missing".to_owned());
        };
        kept.target.render_into(
            renderer,
            &mut kept.damage,
            &pool.moved,
            clear_color,
            region_size,
            pixels,
        )
    })();
    // Emptied whatever happened, so no element -- and no texture it holds --
    // outlives this capture.
    pool.moved.clear();
    match rendered {
        Ok(()) => Some(region),
        Err(error) => {
            // A failed render may have left the target in any state; the
            // next capture starts from a fresh one.
            //
            // Under GLES, dropping it only queues its renderbuffer (see
            // `gles::release_captured`), and `render_into` has already
            // drained. That is bounded without a drain here: it is one
            // cursor-sized renderbuffer per failed render, and the next
            // capture's drain frees it. That drain is `Backend::capture`'s,
            // which `service_captures` runs later in the same tick and the
            // IPC path runs on its next screenshot. So at most one is ever
            // outstanding, and never a frame's worth.
            pool.target = None;
            // warn!: a renderer that cannot draw a cursor-sized region is
            // failing its frames too. The capture is still answered, with the
            // frame's own pixels.
            tracing::warn!(
                %error,
                ?region,
                want,
                "could not draw the cursor into a capture; it shows the frame as drawn"
            );
            None
        }
    }
}

/// The read-back's rows, tightly packed into `out` (cleared first, its
/// capacity reused): the renderer may pad its rows, so the stride is
/// derived from the mapping's length, the same way `screencopy.rs` derives
/// it for the whole frame.
fn pack_rows(pixels: &[u8], width: i32, height: i32, out: &mut Vec<u8>) -> Result<(), String> {
    let row = width as usize * 4;
    let stride = pixels
        .len()
        .checked_div(height.max(1) as usize)
        .unwrap_or(0);
    if width <= 0 || height <= 0 || stride < row {
        return Err(format!(
            "the region read back too small: {} bytes for {width}x{height}",
            pixels.len()
        ));
    }
    out.clear();
    out.reserve(row * height as usize);
    for y in 0..height as usize {
        out.extend_from_slice(&pixels[y * stride..y * stride + row]);
    }
    Ok(())
}

impl State {
    /// The pointer moved, or the cursor's image changed: whatever shows the
    /// cursor has to catch up.
    ///
    /// Where frames draw it (`--tty`), that is a redraw, exactly as before --
    /// but a cursor-only one (`request_cursor_render`), whose damage does
    /// not count as a scene change, so a capture session that did not ask
    /// for the pointer is not re-served an identical picture.
    /// Everywhere else nothing on screen changed, so nothing is rendered --
    /// but a capture session that asked for the pointer (`paint_cursors`)
    /// sees its source change all the same, so [`State::cursor_serial`]
    /// moves (what `screencopy.rs`'s "is this session due" test reads for
    /// such a session), and the frame tick is armed if such a session has a
    /// frame parked, since nothing else would wake it to serve one.
    ///
    /// Every path that moves the pointer or changes its image comes through
    /// here: pointer motion (`input.rs`), `wl_pointer.set_cursor` and
    /// `wp-cursor-shape-v1` (`SeatHandler::cursor_image`), a tablet tool's
    /// cursor (`tablet.rs`), a commit to the cursor surface itself (an
    /// animated cursor, a new hotspot -- `CompositorHandler::commit`), the
    /// cursor surface going away (`destroyed`), a session lock resetting it
    /// to the default shape (`session_lock.rs`), and a reload that rebuilt
    /// the cursor (`reload.rs`).
    ///
    /// Called per motion event, which libinput delivers at up to 1000 Hz: a
    /// counter increment, and either a flag set or a scan of the capture
    /// sessions (none, in a session nobody records), never an allocation.
    /// The tick itself is armed at most once per frame interval.
    pub(crate) fn cursor_changed(&mut self) {
        self.cursor_serial = self.cursor_serial.wrapping_add(1);
        if self.frame_draws_cursor() {
            self.request_cursor_render();
        } else if self.screencopy.cursor_frame_parked() {
            self.ensure_ticking();
        }
    }
}
