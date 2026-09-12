//! The pointer cursor drawn on `--tty` real hardware.
//!
//! `--headless` has no display to draw one on, and `--nested` already shows
//! the host compositor's own cursor on top -- so this backend-neutral module
//! knows nothing about which backend is active; `headless.rs::render` is the
//! one that gates its use to `self.tty.is_some()`.
//!
//! The bitmap is procedurally generated, not an embedded image file or a
//! copy of any cursor theme's actual pixel data -- niri's own cursor assets
//! are GPL, Adwaita's aren't MIT-clean, and flexwm-vision's license note
//! says not to borrow either. It's a plain filled triangle, not a
//! pixel-accurate arrow: this pass is about proving a real
//! position/hotspot/damage-rect pipeline exists at all, not about how the
//! cursor looks -- see this project's standing priority order (bug-free and
//! well-architected before beauty).

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::ImportMem;
use smithay::backend::renderer::Texture;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::input::pointer::CursorImageStatus;
use smithay::utils::{Logical, Physical, Point, Transform};

/// Both dimensions of the square bitmap [`generate_bitmap`] draws.
const SIZE: i32 = 16;

/// A filled right triangle, point at the top-left corner (which is also the
/// hotspot -- see [`Cursor::default`]): a 1px black outline on the left edge
/// and the diagonal, white fill between them, transparent everywhere else.
/// Argb8888, little-endian BGRA byte order -- the same layout `headless.rs`
/// renders into and `tty/buffers.rs` scans out (see their module docs on the
/// same fact), so this needs no conversion anywhere downstream.
fn generate_bitmap() -> Vec<u8> {
    let mut pixels = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let idx = ((y * SIZE + x) * 4) as usize;
            let pixel: [u8; 4] = if x == 0 || x == y {
                [0, 0, 0, 255]
            } else if x < y {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 0]
            };
            pixels[idx..idx + 4].copy_from_slice(&pixel);
        }
    }
    pixels
}

/// The cursor's current image request and its one persistent render buffer.
/// The buffer is built once (see `Default`) and never rebuilt -- same
/// stable-`Id` reasoning as `decorations.rs`'s persistent per-window
/// buffers, so a static cursor doesn't read as "new content" to the damage
/// tracker on every frame it happens to still be visible.
pub struct Cursor {
    buffer: MemoryRenderBuffer,
    /// The point within the bitmap that lines up with the pointer's actual
    /// location -- the top-left corner, since [`generate_bitmap`]'s triangle
    /// has its point there.
    hotspot: Point<i32, Logical>,
    status: CursorImageStatus,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            buffer: MemoryRenderBuffer::from_slice(
                &generate_bitmap(),
                Fourcc::Argb8888,
                (SIZE, SIZE),
                1,
                Transform::Normal,
                None,
            ),
            hotspot: (0, 0).into(),
            status: CursorImageStatus::default_named(),
        }
    }
}

impl Cursor {
    pub fn set_status(&mut self, status: CursorImageStatus) {
        self.status = status;
    }

    /// This frame's cursor render element at `pointer_location`, or `None`
    /// if the cursor is explicitly hidden. `CursorImageStatus::Named` and
    /// `::Surface` both draw the same fallback arrow -- see the module doc
    /// on why drawing a client's actual requested cursor is out of scope for
    /// this pass; hiding the cursor entirely whenever a client sets either
    /// would be a worse regression than showing the wrong shape.
    ///
    /// Generic over the renderer (like `decorations.rs`'s elements) rather
    /// than fixed to `PixmanRenderer`, so this module doesn't need to change
    /// if a future backend uses a different one.
    pub fn element<R>(
        &self,
        renderer: &mut R,
        pointer_location: Point<f64, Logical>,
    ) -> Result<Option<MemoryRenderBufferRenderElement<R>>, R::Error>
    where
        R: ImportMem,
        R::TextureId: Texture + Send + Clone + 'static,
    {
        if matches!(self.status, CursorImageStatus::Hidden) {
            return Ok(None);
        }
        let location = element_location(pointer_location, self.hotspot);
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            location,
            &self.buffer,
            None,
            None,
            None,
            Kind::Cursor,
        )
        .map(Some)
    }
}

/// Where the render element's origin belongs given the pointer's own
/// location and the bitmap's hotspot -- pulled out of `element` so this
/// arithmetic is testable without a live renderer, same rationale as
/// `input.rs`'s `clamp_to_extent`. Physical, not Logical, matches what
/// `MemoryRenderBufferRenderElement::from_buffer` wants for `location`
/// (renderer output space) -- numerically identical to Logical here since
/// this project never runs an output at a scale other than 1.0.
fn element_location(
    pointer: Point<f64, Logical>,
    hotspot: Point<i32, Logical>,
) -> Point<f64, Physical> {
    Point::from((
        pointer.x - f64::from(hotspot.x),
        pointer.y - f64::from(hotspot.y),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmap_hotspot_pixel_is_opaque_black() {
        let pixels = generate_bitmap();
        assert_eq!(&pixels[0..4], &[0, 0, 0, 255]);
    }

    #[test]
    fn bitmap_is_fully_transparent_above_the_diagonal() {
        let pixels = generate_bitmap();
        // (SIZE - 1, 0): far right of the top row, well outside the triangle.
        let idx = ((SIZE - 1) * 4) as usize;
        assert_eq!(&pixels[idx..idx + 4], &[0, 0, 0, 0]);
    }

    #[test]
    fn bitmap_interior_is_opaque_white() {
        let pixels = generate_bitmap();
        // (1, 3): strictly inside the triangle (0 < x < y).
        let idx = ((3 * SIZE + 1) * 4) as usize;
        assert_eq!(&pixels[idx..idx + 4], &[255, 255, 255, 255]);
    }

    #[test]
    fn a_zero_hotspot_leaves_the_pointer_location_unchanged() {
        let pointer = Point::<f64, Logical>::from((12.5, 30.0));
        let hotspot = Point::<i32, Logical>::from((0, 0));
        let location = element_location(pointer, hotspot);
        assert_eq!((location.x, location.y), (12.5, 30.0));
    }

    #[test]
    fn a_nonzero_hotspot_offsets_the_pointer_location() {
        let pointer = Point::<f64, Logical>::from((100.0, 100.0));
        let hotspot = Point::<i32, Logical>::from((4, 6));
        let location = element_location(pointer, hotspot);
        assert_eq!((location.x, location.y), (96.0, 94.0));
    }
}
