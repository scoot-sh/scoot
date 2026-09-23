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
//! the region the mismatch covers -- where the cursor sits now, plus where
//! the recorded frame composited it, clamped to the output -- is rendered
//! again from the frame's own element list, with the cursor or without it,
//! into an offscreen target of that region's size, by the same renderer that
//! draws the output's frames. The result replaces that region of the
//! captured copy.
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
//!   over it, and the swapchain slot a capture reads holds that hole. The
//!   re-rendered region covers it either way.
//! - **Same pixels the screen shows.** The cursor comes from the same
//!   `Cursor::element` call the frame makes -- image, hotspot, scale -- and
//!   the region is drawn by the same renderer from the same element list,
//!   moved by a whole number of pixels. Relocation changes no sampling, so
//!   the region matches what a full composite would have drawn there.
//!
//! It is also bounded: the region is the cursor's size (tens of pixels
//! square), not the output's. The worst case is a client cursor surface as
//! large as the output, which costs one full-output render per capture --
//! the ceiling the whole-frame alternative would have paid every time.
//!
//! # What "the recorded frame holds the cursor" is read from
//!
//! Not guessed from the tier. Each frame records a [`CursorInFrame`] beside
//! the pixels a capture reads: where the cursor elements it composited
//! landed, and whether any of them reached the screen some other way (a KMS
//! plane). The dumb tier and the offscreen renderers record it from the list
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
use smithay::backend::renderer::{
    Bind, ExportMem, ImportAll, ImportMem, Offscreen, Renderer, Texture,
};
use smithay::output::Output;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Transform};

use scoot_core::OutputId;

use super::elements::{Elements, FrameContext, ring_elements};
use super::{Backend, Pipeline, frame_clear_color, read_back};
use crate::compositor::State;
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
    /// Whether some cursor element of the frame reached the screen without
    /// being composited into it -- on a KMS cursor or overlay plane. Such a
    /// frame's pixels lack (part of) the cursor, and, over an underlay, may
    /// hold a transparent hole where it sits.
    pub(crate) off_frame: bool,
}

impl CursorInFrame {
    /// The record for a frame drawn from `elements`, where `on_plane` says
    /// which elements reached the screen other than through the frame.
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
        on_plane: impl Fn(&Id) -> bool,
    ) -> Self {
        let mut record = Self::default();
        for element in elements {
            if element.kind() != Kind::Cursor {
                continue;
            }
            if on_plane(element.id()) {
                record.off_frame = true;
                continue;
            }
            let Some(visible) = clamp(element.geometry(scale), bounds) else {
                continue;
            };
            record.composited = union(record.composited, Some(visible));
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
/// - **Asked for.** Nothing to do only when the frame composited the whole
///   cursor exactly where it is now. Anything else -- a plane took some of
///   it, the frame never drew it (headless, nested), or it moved since --
///   re-renders both places, so the old one loses its stale pixels and the
///   new one gains the cursor.
/// - **Not asked for.** Only a frame that composited cursor pixels needs
///   anything, and only where it composited them; where the cursor is now
///   holds whatever the frame drew there, which is right without it.
///
/// Pure, so every combination is pinned without a renderer.
pub(super) fn patch_region(
    recorded: CursorInFrame,
    now: Option<Rectangle<i32, Physical>>,
    want: bool,
) -> Option<Rectangle<i32, Physical>> {
    if !want {
        return recorded.composited;
    }
    if recorded.composited == now && !recorded.off_frame {
        return None;
    }
    union(recorded.composited, now)
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
    PatchElement<R> where R: ImportAll + ImportMem;
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

impl State {
    /// The region of output `id`'s next capture that has to be re-rendered
    /// for it to show the cursor exactly as `want` asks, rendered -- or
    /// `None` when the frame the capture reads already does (or the region
    /// could not be rendered, which is logged and leaves the capture as
    /// read).
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
    /// render of a cursor-sized region, plus that region's read-back. The
    /// target and the read-back are allocated per call rather than pooled:
    /// both are cursor-sized, beside a capture whose own read-back
    /// allocates a copy of the whole output every time.
    pub(crate) fn capture_cursor_patch(
        &mut self,
        backend: &mut Backend,
        id: OutputId,
        want: bool,
    ) -> Option<CursorPatch> {
        let recorded = backend.cursor_in_frame();
        // Nothing to take out of a frame that composited no cursor: no
        // gather, no renderer.
        if !want && recorded.composited.is_none() {
            return None;
        }
        let output = self.outputs.get(id)?.clone();
        let size = backend.size;
        match &mut backend.pipeline {
            Pipeline::Pixman(cpu) => render_patch::<_, pixman::Image<'static, 'static>>(
                self,
                &mut cpu.renderer,
                size,
                &output,
                recorded,
                want,
            ),
            Pipeline::Gles(gpu) => render_patch::<
                _,
                smithay::backend::renderer::gles::GlesRenderbuffer,
            >(
                self, &mut gpu.renderer, size, &output, recorded, want
            ),
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(gpu) => render_patch::<
                _,
                smithay::backend::renderer::gles::GlesRenderbuffer,
            >(
                self, &mut gpu.renderer, size, &output, recorded, want
            ),
        }
    }
}

/// [`State::capture_cursor_patch`]'s body over whichever renderer the
/// backend carries, rendering into an offscreen target of type `T`.
///
/// Gathers exactly as the frame does (same `locked`, same arrangement, same
/// ring), with the cursor always in the list so its footprint is known --
/// then drops it again when `want` is false.
fn render_patch<R, T>(
    state: &mut State,
    renderer: &mut R,
    size: (i32, i32),
    output: &Output,
    recorded: CursorInFrame,
    want: bool,
) -> Option<CursorPatch>
where
    R: Renderer + ImportAll + ImportMem + Offscreen<T> + Bind<T> + ExportMem,
    R::TextureId: Texture + Send + Clone + 'static,
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
    let now = CursorInFrame::of(
        &state.cursor_elements(renderer, &frame),
        scale,
        bounds,
        |_| false,
    )
    .composited;
    let region = patch_region(recorded, now, want)?;

    let arrangement = if locked {
        None
    } else {
        Some(state.world.arrange())
    };
    let ring = ring_elements(
        &mut state.decorations,
        &state.appearance,
        arrangement.as_ref(),
        &frame,
        renderer,
    );
    let (elements, _) =
        state.gather_elements(renderer, output, &frame, ring, arrangement.as_ref(), true);
    let by = Point::from((-region.loc.x, -region.loc.y));
    let moved: Vec<PatchElement<R>> = elements
        .into_iter()
        .filter(|element| want || element.kind() != Kind::Cursor)
        .map(|element| relocate(element, by))
        .collect();
    let clear_color = frame_clear_color(state, locked);

    let (width, height) = (region.size.w, region.size.h);
    let rendered = (|| -> Result<Vec<u8>, String> {
        let mut target: T = renderer
            .create_buffer(Fourcc::Argb8888, (width, height).into())
            .map_err(|error| format!("could not allocate the region: {error}"))?;
        let mut framebuffer = renderer
            .bind(&mut target)
            .map_err(|error| format!("could not bind the region: {error}"))?;
        let mut damage = OutputDamageTracker::new((width, height), scale, Transform::Normal);
        damage
            .render_output(renderer, &mut framebuffer, 0, &moved, clear_color)
            .map_err(|error| format!("could not render the region: {error:?}"))?;
        let whole: Rectangle<i32, Buffer> = Rectangle::from_size((width, height).into());
        read_back(renderer, &framebuffer, whole, |pixels| {
            pack_rows(pixels, width, height)
        })
        .map_err(|error| format!("could not read the region back: {error}"))?
    })();
    match rendered {
        Ok(pixels) => Some(CursorPatch {
            rect: region,
            pixels,
        }),
        Err(error) => {
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

/// The read-back's rows, tightly packed: the renderer may pad its rows, so
/// the stride is derived from the mapping's length, the same way
/// `screencopy.rs` derives it for the whole frame.
fn pack_rows(pixels: &[u8], width: i32, height: i32) -> Result<Vec<u8>, String> {
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
    let mut packed = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        packed.extend_from_slice(&pixels[y * stride..y * stride + row]);
    }
    Ok(packed)
}

impl State {
    /// The pointer moved, or the cursor's image changed: whatever shows the
    /// cursor has to catch up.
    ///
    /// Where frames draw it (`--tty`), that is a redraw, exactly as before.
    /// Everywhere else nothing on screen changed, so nothing is rendered --
    /// but a capture session that asked for the pointer (`paint_cursors`)
    /// sees its source change all the same, so [`State::cursor_serial`]
    /// moves (what `screencopy.rs`'s "is this session due" test reads for
    /// such a session), and the frame tick is armed if such a session has a
    /// frame parked, since nothing else would wake it to serve one.
    ///
    /// Called per motion event, which libinput delivers at up to 1000 Hz: a
    /// counter increment, and either a flag set or a scan of the capture
    /// sessions (none, in a session nobody records), never an allocation.
    /// The tick itself is armed at most once per frame interval.
    pub(crate) fn cursor_changed(&mut self) {
        self.cursor_serial = self.cursor_serial.wrapping_add(1);
        if self.frame_draws_cursor() {
            self.request_render();
        } else if self.screencopy.cursor_frame_parked() {
            self.ensure_ticking();
        }
    }
}
