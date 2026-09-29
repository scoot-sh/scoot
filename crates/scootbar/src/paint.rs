//! Pixels: the drawing the bar does, with no Wayland objects, so it is
//! tested on plain byte slices. The skeleton fills the bar with one color;
//! text, rectangles and per-module damage arrive with
//! `docs/scootbar/backlog/module-api-and-clock.md`, and grow here into the
//! pure canvas `docs/scootbar/backlog/extract-scootui.md` would extract.

use crate::color::Color;

#[cfg(test)]
mod tests;

/// Fills `pixels`, an `XRGB8888` buffer (4 bytes a pixel, little-endian),
/// with `color`. A trailing partial pixel (never there in a buffer whose
/// length is `stride × height`) is left alone rather than half-written.
pub fn fill(pixels: &mut [u8], color: Color) {
    let pixel = color.xrgb8888().to_le_bytes();
    for chunk in pixels.chunks_exact_mut(4) {
        chunk.copy_from_slice(&pixel);
    }
}
