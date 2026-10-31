//! The pointer cursor's own image, for `ext-image-copy-capture-v1` pointer
//! cursor sessions (`screencopy/cursor_session.rs`).
//!
//! A capture that asked for `paint_cursors` gets the cursor composited into
//! the screen's pixels (the rest of this module's parent). A pointer cursor
//! session is the other shape: the cursor *alone*, in a buffer of its own,
//! with its hotspot, so a remote-desktop server can send the shape separately
//! from the screen and the viewer draws the pointer locally.
//!
//! # How: the same elements, rendered alone
//!
//! The image is what the frame would draw for the cursor -- the very
//! `Cursor::element` call, at the output's scale -- rendered over a
//! transparent clear into a target the size of the elements' bounding box.
//! Size and hotspot come from those elements, not from the shape's metadata,
//! so a drawn shape, a theme image and a client's cursor surface tree (with
//! subsurfaces at negative offsets) all follow one rule and the size can
//! never disagree with the pixels. The hotspot is where the pointer sits
//! inside that box.
//!
//! It reuses the capture path's pooled target and damage tracker
//! ([`PatchPool`]) and draws through [`draw_pooled`], so an image render
//! allocates nothing a cursor-region render would not, and runs only when the
//! cursor's image changed -- never per frame, never per pointer move.

use smithay::backend::renderer::element::Element;
use smithay::backend::renderer::{Color32F, ImportAll, ImportMem, Renderer, Texture};
use smithay::output::Output;
use smithay::utils::{Physical, Point, Rectangle, Scale};

use scoot_core::OutputId;

use super::super::elements::{Elements, FrameContext};
use super::super::{Backend, Pipeline};
use super::{PatchPool, PatchTarget, clamp, draw_pooled, relocate, union};
use crate::compositor::State;

/// The longest edge of a cursor image, in physical pixels.
///
/// A client's cursor surface can be as large as it likes (a viewport makes
/// one billions of pixels wide), and the image is allocated and sent over the
/// wire. Real cursors are tens of pixels; 512 holds the largest themed
/// cursor at a 4x output scale. A larger surface is cropped to its top-left
/// 512x512, hotspot kept in image coordinates.
pub(crate) const MAX_EDGE: i32 = 512;

/// A rendered cursor image: its size and hotspot, with the pixels in the
/// buffer the caller passed (tightly packed little-endian `Argb8888`,
/// premultiplied alpha as the format means).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CursorImage {
    pub(crate) size: (i32, i32),
    /// The pointer's position inside the image, in image pixels. Outside the
    /// image only for a client cursor whose hotspot lies off its own surface.
    pub(crate) hotspot: (i32, i32),
}

impl CursorImage {
    /// `size` fully transparent pixels written to `pixels`: what a hidden
    /// cursor, a cursor surface with no content yet, and a failed render all
    /// answer, so the client's pointer image goes away rather than going
    /// stale.
    pub(crate) fn transparent(size: (i32, i32), pixels: &mut Vec<u8>) -> Self {
        let size = (size.0.clamp(1, MAX_EDGE), size.1.clamp(1, MAX_EDGE));
        pixels.clear();
        pixels.resize(size.0 as usize * size.1 as usize * 4, 0);
        Self {
            size,
            hotspot: (0, 0),
        }
    }
}

impl State {
    /// Renders the cursor as it looks on output `id` into `pixels`, with the
    /// pointer's hotspot at `at` (output physical pixels).
    ///
    /// `fallback` is the size to answer when there is nothing to draw -- the
    /// last size the clients were told, so a cursor hiding and reappearing
    /// does not churn their buffers. A renderer failure answers transparent
    /// at that size too, logged, and never an error: the client keeps its
    /// stream and the next change re-renders.
    pub(crate) fn render_cursor_image(
        &self,
        backend: &mut Backend,
        id: OutputId,
        at: Point<i32, Physical>,
        fallback: (i32, i32),
        pixels: &mut Vec<u8>,
    ) -> CursorImage {
        let Some(output) = self.outputs.get(id) else {
            return CursorImage::transparent(fallback, pixels);
        };
        let size = backend.size;
        match &mut backend.pipeline {
            Pipeline::Pixman(cpu) => render_image(
                self,
                &mut cpu.renderer,
                &mut cpu.patch,
                size,
                output,
                at,
                fallback,
                pixels,
            ),
            Pipeline::Gles(gpu) => {
                let super::super::gles::GlesBackend {
                    renderer, patch, ..
                } = &mut **gpu;
                render_image(self, renderer, patch, size, output, at, fallback, pixels)
            }
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(gpu) => {
                let super::super::scanout::ScanoutBackend {
                    renderer, patch, ..
                } = &mut **gpu;
                render_image(self, renderer, patch, size, output, at, fallback, pixels)
            }
        }
    }
}

/// [`State::render_cursor_image`]'s body over whichever renderer the backend
/// carries.
#[allow(clippy::too_many_arguments)]
fn render_image<R, T>(
    state: &State,
    renderer: &mut R,
    pool: &mut PatchPool<R, T>,
    size: (i32, i32),
    output: &Output,
    at: Point<i32, Physical>,
    fallback: (i32, i32),
    pixels: &mut Vec<u8>,
) -> CursorImage
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Send + Clone + 'static,
    T: PatchTarget<R>,
{
    let frame = FrameContext {
        size,
        scale: output.current_scale().fractional_scale(),
        geometry: state.space.output_geometry(output),
        output: state.outputs.id_of(output),
        locked: state.session_lock.is_locked(),
        // Only the cursor is gathered from this context.
        gesture_dx: 0,
    };
    let scale = Scale::from(frame.scale);
    pool.cursor.clear();
    state.cursor_elements_into(renderer, &frame, &mut pool.cursor);
    // The box the elements cover, each clamped to a window around the output
    // wide enough for any real cursor: a hostile surface size never reaches
    // the union's arithmetic (the same reason `CursorInFrame::of` clamps),
    // while a cursor hanging off the output's edge still counts whole.
    let window = Rectangle::new(
        (-MAX_EDGE, -MAX_EDGE).into(),
        (
            size.0.saturating_add(2 * MAX_EDGE),
            size.1.saturating_add(2 * MAX_EDGE),
        )
            .into(),
    );
    let mut covered = None;
    for element in &pool.cursor {
        covered = union(covered, clamp(element.geometry(scale), window));
    }
    let Some(covered) = covered else {
        pool.cursor.clear();
        return CursorImage::transparent(fallback, pixels);
    };
    let image_size = (covered.size.w.min(MAX_EDGE), covered.size.h.min(MAX_EDGE));
    let by = Point::from((-covered.loc.x, -covered.loc.y));
    pool.moved.extend(
        pool.cursor
            .drain(..)
            .map(|element| relocate(Elements::Cursor(element), by)),
    );
    match draw_pooled(
        renderer,
        pool,
        image_size,
        frame.scale,
        Color32F::TRANSPARENT,
        pixels,
    ) {
        Ok(()) => CursorImage {
            size: image_size,
            hotspot: (at.x - covered.loc.x, at.y - covered.loc.y),
        },
        Err(error) => {
            // A failed render may have left the target in any state; the
            // next one starts from a fresh one (see `render_patch`).
            pool.target = None;
            tracing::warn!(
                %error,
                ?image_size,
                "could not draw the pointer cursor's image; serving it transparent"
            );
            CursorImage::transparent(fallback, pixels)
        }
    }
}
