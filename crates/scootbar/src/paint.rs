//! Pixels: the drawing the bar does, with no Wayland objects, so it is
//! tested on plain byte slices. A [`Canvas`] over an `XRGB8888` or
//! premultiplied `ARGB8888` buffer (4 bytes a pixel, little-endian), with
//! full-height span fills, glyph coverage blended over what is there, and the
//! bar's rounded corners. This is the pure canvas
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

/// The bar's rounded corners: the coverage (0 to 255, how much of the pixel
/// is inside the bar) of the top-left quadrant, `radius` × `radius` pixels,
/// which the other three mirror. Analytic, no supersampling: a pixel's
/// coverage is how far its center sits inside the circle, as a fraction of
/// a pixel. Built once per radius (the scene caches it), read every time
/// the background is painted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Corners {
    radius: u32,
    coverage: Vec<u8>,
}

impl Corners {
    /// Square corners: nothing is cut.
    pub const NONE: Self = Self {
        radius: 0,
        coverage: Vec::new(),
    };

    /// Corners of `radius` device pixels.
    pub fn new(radius: u32) -> Self {
        let r = radius as usize;
        let mut coverage = Vec::with_capacity(r * r);
        let edge = radius as f32;
        for y in 0..r {
            for x in 0..r {
                // The pixel's center, from the circle's center.
                let dx = edge - (x as f32 + 0.5);
                let dy = edge - (y as f32 + 0.5);
                let inside = edge - dx.hypot(dy) + 0.5;
                coverage.push((inside.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        Self { radius, coverage }
    }

    pub fn radius(&self) -> u32 {
        self.radius
    }

    /// The coverage of quadrant pixel `(x, y)`, from the corner; 0 outside
    /// the table.
    fn at(&self, x: usize, y: usize) -> u8 {
        let r = self.radius as usize;
        if x >= r {
            return 255;
        }
        self.coverage.get(y * r + x).copied().unwrap_or(255)
    }
}

/// `pixel` (`b, g, r, a`, premultiplied) with every channel scaled by
/// `coverage` out of 255, rounded.
fn scaled(pixel: [u8; 4], coverage: u8) -> [u8; 4] {
    let scale = |c: u8| ((u32::from(c) * u32::from(coverage) + 127) / 255) as u8;
    pixel.map(scale)
}

/// An `XRGB8888` or premultiplied `ARGB8888` image `width` × `height`, rows packed (`stride = width ×
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
    #[allow(dead_code)] // Only the workspaces module reads it; the bar fills through `fill_shaped`.
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

    /// Fills the full-height `span`, clipped to the canvas, with `color` at
    /// `alpha` (premultiplied), less the pixels `corners` cuts from the
    /// canvas's four corners (partly, at their edge). With square corners,
    /// or a canvas too small to hold them (`2 × radius` past a side), this
    /// is a plain fill.
    pub fn fill_shaped(&mut self, span: Span, color: Color, alpha: u8, corners: &Corners) {
        let x0 = span.x.min(self.width) as usize;
        let x1 = span.end().min(self.width) as usize;
        if x0 >= x1 {
            return;
        }
        let pixel = color.argb8888(alpha).to_le_bytes();
        let (width, height) = (self.width as usize, self.height as usize);
        let radius = corners.radius as usize;
        // Rows within `radius` of the top or bottom, and only if the
        // corners fit.
        let cut = if radius > 0 && radius * 2 <= width.min(height) {
            radius
        } else {
            0
        };
        let stride = width * 4;
        for (y, row) in self
            .pixels
            .chunks_exact_mut(stride)
            .take(height)
            .enumerate()
        {
            let Some(run) = row.get_mut(x0 * 4..x1 * 4) else {
                continue;
            };
            for chunk in run.chunks_exact_mut(4) {
                chunk.copy_from_slice(&pixel);
            }
            if cut == 0 || (y >= cut && y < height - cut) {
                continue;
            }
            let from_edge = y.min(height - 1 - y);
            for x in x0..x1.min(cut) {
                let at = x * 4;
                row[at..at + 4].copy_from_slice(&scaled(pixel, corners.at(x, from_edge)));
            }
            for x in x0.max(width - cut)..x1 {
                let at = x * 4;
                row[at..at + 4]
                    .copy_from_slice(&scaled(pixel, corners.at(width - 1 - x, from_edge)));
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
        // Premultiplied: the alpha channel is blended like the others,
        // with an opaque source (a no-op on an opaque pixel, and the pad
        // byte of `XRGB8888` is not read).
        *pad = mix(0xff, *pad);
    }

    /// The alpha byte at `(x, y)`, for tests.
    #[cfg(test)]
    pub fn alpha_at(&self, x: u32, y: u32) -> u8 {
        self.pixels[(y as usize * self.width as usize + x as usize) * 4 + 3]
    }

    /// The pixel at `(x, y)` as `[r, g, b]`, for tests.
    #[cfg(test)]
    pub fn at(&self, x: u32, y: u32) -> [u8; 3] {
        let index = (y as usize * self.width as usize + x as usize) * 4;
        let p = &self.pixels[index..index + 4];
        [p[2], p[1], p[0]]
    }
}
