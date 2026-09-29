//! Pixels: the drawing the bar does, with no Wayland objects, so it is
//! tested on plain byte slices. A [`Canvas`] over an `XRGB8888` buffer
//! (4 bytes a pixel, little-endian), with full-height span fills and glyph
//! coverage blended over what is there. This is the pure canvas
//! `docs/scootbar/backlog/extract-scootui.md` would extract.

use crate::color::Color;

#[cfg(test)]
mod tests;

/// A horizontal span of the bar: `x .. x + width`, in device pixels. Each
/// module owns one, full height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub x: u32,
    pub width: u32,
}

impl Span {
    pub fn end(self) -> u32 {
        self.x.saturating_add(self.width)
    }
}

/// An `XRGB8888` image `width` × `height`, rows packed (`stride = width ×
/// 4`). Every access is bounds-checked: a draw outside it is clipped, never
/// a panic.
pub struct Canvas<'a> {
    pixels: &'a mut [u8],
    width: u32,
    height: u32,
}

impl<'a> Canvas<'a> {
    /// `None` if `pixels` is shorter than `width × height` pixels.
    pub fn new(pixels: &'a mut [u8], width: u32, height: u32) -> Option<Self> {
        let needed = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        (pixels.len() >= needed).then_some(Self {
            pixels,
            width,
            height,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Fills the full-height `span`, clipped to the canvas, with `color`.
    pub fn fill_span(&mut self, span: Span, color: Color) {
        let x0 = span.x.min(self.width) as usize;
        let x1 = span.end().min(self.width) as usize;
        if x0 >= x1 {
            return;
        }
        let pixel = color.xrgb8888().to_le_bytes();
        let stride = self.width as usize * 4;
        for row in self
            .pixels
            .chunks_exact_mut(stride)
            .take(self.height as usize)
        {
            if let Some(run) = row.get_mut(x0 * 4..x1 * 4) {
                for chunk in run.chunks_exact_mut(4) {
                    chunk.copy_from_slice(&pixel);
                }
            }
        }
    }

    /// Blends `color` at `coverage` (0 to 255) over the pixel at `(x, y)`,
    /// if it is inside `clip` and the canvas.
    pub fn blend(&mut self, x: i64, y: i64, coverage: u8, color: Color, clip: Span) {
        if coverage == 0
            || x < i64::from(clip.x)
            || x >= i64::from(clip.end())
            || x < 0
            || y < 0
            || x >= i64::from(self.width)
            || y >= i64::from(self.height)
        {
            return;
        }
        // Both within the canvas, whose size in bytes is a `usize`.
        let index = (y as usize * self.width as usize + x as usize) * 4;
        let Some([blue, green, red, pad]) = self.pixels.get_mut(index..index + 4) else {
            return;
        };
        let alpha = u32::from(coverage);
        let mix = |over: u8, under: u8| -> u8 {
            // Rounded (a × c + b × (255 − c)) / 255: at most 255.
            ((u32::from(over) * alpha + u32::from(under) * (255 - alpha) + 127) / 255) as u8
        };
        *blue = mix(color.b, *blue);
        *green = mix(color.g, *green);
        *red = mix(color.r, *red);
        *pad = 0xff;
    }

    /// The pixel at `(x, y)` as `[r, g, b]`, for tests.
    #[cfg(test)]
    pub fn at(&self, x: u32, y: u32) -> [u8; 3] {
        let index = (y as usize * self.width as usize + x as usize) * 4;
        let p = &self.pixels[index..index + 4];
        [p[2], p[1], p[0]]
    }
}
