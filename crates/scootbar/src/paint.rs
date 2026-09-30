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
                coverage.push(corner_coverage(edge, dx, dy));
            }
        }
        Self { radius, coverage }
    }

    pub fn radius(&self) -> u32 {
        self.radius
    }

    /// The coverage at quadrant pixel `(x, y)`, for the region tests.
    #[cfg(test)]
    pub fn coverage_at(&self, x: u32, y: u32) -> u8 {
        self.at(x as usize, y as usize)
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

/// The coverage (0 to 255) of a pixel whose center is `dx` and `dy` from the
/// center of a circle of `radius`: how far inside it sits, in pixels,
/// clamped to one. The one formula the bar's corners and the pill share.
fn corner_coverage(radius: f32, dx: f32, dy: f32) -> u8 {
    let inside = radius - dx.hypot(dy) + 0.5;
    (inside.clamp(0.0, 1.0) * 255.0).round() as u8
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

    /// Fills rows `y0 .. y1` of `span`, both clipped to the canvas, with
    /// `color`, opaque: the separators' lines.
    pub fn fill_rect(&mut self, span: Span, y0: u32, y1: u32, color: Color) {
        let x0 = span.x.min(self.width) as usize;
        let x1 = span.end().min(self.width) as usize;
        let (y0, y1) = (y0.min(self.height) as usize, y1.min(self.height) as usize);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let pixel = color.xrgb8888().to_le_bytes();
        let stride = self.width as usize * 4;
        for row in self.pixels.chunks_exact_mut(stride).take(y1).skip(y0) {
            if let Some(run) = row.get_mut(x0 * 4..x1 * 4) {
                for chunk in run.chunks_exact_mut(4) {
                    chunk.copy_from_slice(&pixel);
                }
            }
        }
    }

    /// Fills the rounded rectangle `span` x rows `y0 .. y1`, clipped to the
    /// canvas, with `color`: corners of `radius` (cut back to what it can
    /// hold), antialiased over what is there, the rest opaque. The
    /// active-workspace pill. No allocation: a corner pixel's coverage is
    /// computed as [`Corners::new`] does, only for the few in the corners,
    /// and everything else is whole-row runs.
    #[allow(dead_code)] // Only the workspaces module draws a pill.
    pub fn fill_pill(&mut self, span: Span, y0: u32, y1: u32, radius: u32, color: Color) {
        let (w, h) = (span.width, y1.saturating_sub(y0));
        let radius = radius.min(w / 2).min(h / 2);
        if radius == 0 {
            self.fill_rect(span, y0, y1, color);
            return;
        }
        // The straight rows between the corners, in one fill.
        self.fill_rect(
            span,
            y0.saturating_add(radius),
            y1.saturating_sub(radius),
            color,
        );
        let r = radius as f32;
        let pixel = color.xrgb8888().to_le_bytes();
        let clip = Span {
            x: 0,
            width: self.width,
        };
        for ly in (0..radius).chain(h - radius..h) {
            let y = y0.saturating_add(ly);
            if y >= self.height {
                break;
            }
            // The row's distance from the arcs' center row.
            let dy = if ly < radius {
                r - (ly as f32 + 0.5)
            } else {
                (ly as f32 + 0.5) - (h - radius) as f32
            };
            // Between the corners the row is whole.
            self.fill_rect(
                Span {
                    x: span.x.saturating_add(radius),
                    width: w - 2 * radius,
                },
                y,
                y.saturating_add(1),
                color,
            );
            for lx in (0..radius).chain(w - radius..w) {
                let x = span.x.saturating_add(lx);
                if x >= self.width {
                    break;
                }
                let dx = if lx < radius {
                    r - (lx as f32 + 0.5)
                } else {
                    (lx as f32 + 0.5) - (w - radius) as f32
                };
                match corner_coverage(r, dx, dy) {
                    0 => {}
                    255 => {
                        let at = (y as usize * self.width as usize + x as usize) * 4;
                        if let Some(dest) = self.pixels.get_mut(at..at + 4) {
                            dest.copy_from_slice(&pixel);
                        }
                    }
                    coverage => self.blend(i64::from(x), i64::from(y), coverage, color, clip),
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

    /// Blends `pixel` (`b, g, r, a`, premultiplied: an image's) over the
    /// pixel at `(x, y)`, if it is inside `clip` and the canvas: the
    /// source-over `src + dst × (255 − a) ÷ 255` on every channel.
    pub fn blend_premultiplied(&mut self, x: i64, y: i64, pixel: [u8; 4], clip: Span) {
        if pixel[3] == 0
            || x < i64::from(clip.x)
            || x >= i64::from(clip.end())
            || x < 0
            || y < 0
            || x >= i64::from(self.width)
            || y >= i64::from(self.height)
        {
            return;
        }
        let index = (y as usize * self.width as usize + x as usize) * 4;
        let Some(dest) = self.pixels.get_mut(index..index + 4) else {
            return;
        };
        let keep = 255 - u32::from(pixel[3]);
        for (d, &s) in dest.iter_mut().zip(&pixel) {
            // At most `s + d`, which a valid premultiplied pixel keeps
            // within 255; clamped in case the source is not valid.
            *d = (u32::from(s) + (u32::from(*d) * keep + 127) / 255).min(255) as u8;
        }
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
