//! A card painted once, composited many times: the off-screen surface a
//! moving card is drawn through.
//!
//! A card in motion is rasterized into a [`Layer`] when its motion begins
//! (text, border, shadow, icon: the expensive part) and then composited
//! onto the canvas on every frame with an offset, a scale about its center
//! and a global alpha: a pure blit, no glyph work. Layers are premultiplied
//! `ARGB8888`, like the canvas, so a composite is one source-over blend.
//!
//! A layer owns its pixels and reuses them: [`Layer::begin`] reshapes it
//! and clears it without freeing, so a pooled layer allocates only when a
//! card is larger than any it has held. [`Layer::composite`] allocates
//! nothing and clips to the canvas.

use super::Canvas;

/// The most pixels a layer holds (8 MiB of `ARGB8888`): a card larger than
/// this is not worth moving and is drawn in place instead. The shell's
/// cards are a few hundred pixels across.
pub const MAX_LAYER_PIXELS: usize = 2 * 1024 * 1024;

/// A reusable off-screen premultiplied `ARGB8888` image.
#[derive(Debug, Default)]
pub struct Layer {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

impl Layer {
    /// An empty layer: holds no pixels until [`Layer::begin`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Reshapes the layer to `width` × `height` and clears it to
    /// transparent, keeping its allocation when it is large enough.
    /// Returns false (and leaves the layer empty) for a size that is zero
    /// or larger than [`MAX_LAYER_PIXELS`].
    pub fn begin(&mut self, width: u32, height: u32) -> bool {
        self.width = 0;
        self.height = 0;
        let pixels = (width as usize).saturating_mul(height as usize);
        if pixels == 0 || pixels > MAX_LAYER_PIXELS {
            return false;
        }
        self.pixels.clear();
        self.pixels.resize(pixels * 4, 0);
        self.width = width;
        self.height = height;
        true
    }

    /// The layer's width in pixels; zero while empty.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The layer's height in pixels; zero while empty.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Whether the layer holds an image.
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// A canvas over the layer to paint the card into, in layer
    /// coordinates. `None` while the layer is empty.
    pub fn canvas(&mut self) -> Option<Canvas<'_>> {
        if self.is_empty() {
            return None;
        }
        Canvas::new(&mut self.pixels, self.width, self.height)
    }

    /// Composites the layer onto `dst`, source-over. `(x, y)` is where the
    /// layer's top-left lands when `scale` is `1.0`; any other scale
    /// shrinks or grows the image about its center. `alpha` (0 to 255) is
    /// a global opacity on top of the layer's own. Clipped to `dst`; a
    /// scale at or below zero, or a zero alpha, draws nothing.
    pub fn composite(&self, dst: &mut Canvas<'_>, x: i32, y: i32, scale: f32, alpha: u8) {
        if self.is_empty() || alpha == 0 || !scale.is_finite() || scale <= 0.0 {
            return;
        }
        let (w, h) = (i64::from(self.width), i64::from(self.height));
        // The destination size and corner, scaled about the center. A
        // scale within a 256th of one is one: the unscaled copy is exact.
        let unscaled = (scale - 1.0).abs() < 1.0 / 256.0;
        let (dw, dh) = if unscaled {
            (w, h)
        } else {
            (
                ((w as f64 * f64::from(scale)).round() as i64).clamp(1, 8192),
                ((h as f64 * f64::from(scale)).round() as i64).clamp(1, 8192),
            )
        };
        let left = i64::from(x) + (w - dw).div_euclid(2);
        let top = i64::from(y) + (h - dh).div_euclid(2);
        let (cw, ch) = (i64::from(dst.width), i64::from(dst.height));
        let (x0, x1) = (left.max(0), (left + dw).min(cw));
        let (y0, y1) = (top.max(0), (top + dh).min(ch));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let g = u32::from(alpha);
        let src_stride = self.width as usize * 4;
        let dst_stride = dst.width as usize * 4;
        for row in y0..y1 {
            let Some(out) = dst
                .pixels
                .get_mut(row as usize * dst_stride + x0 as usize * 4..)
                .and_then(|rest| rest.get_mut(..(x1 - x0) as usize * 4))
            else {
                continue;
            };
            if unscaled {
                let sy = (row - top) as usize;
                let from = sy * src_stride + (x0 - left) as usize * 4;
                let Some(src) = self
                    .pixels
                    .get(from..)
                    .and_then(|rest| rest.get(..out.len()))
                else {
                    continue;
                };
                for (d, s) in out.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    over(d, [s[0], s[1], s[2], s[3]], g);
                }
            } else {
                // Bilinear in 16.16 fixed point: the source position of a
                // destination pixel center, minus half a source pixel.
                let step_x = (w << 16) / dw;
                let step_y = (h << 16) / dh;
                let fy = (row - top) * step_y + step_y / 2 - 0x8000;
                let (ty0, ty1, wy) = taps(fy, h);
                for (offset, d) in out.chunks_exact_mut(4).enumerate() {
                    let ox = x0 - left + offset as i64;
                    let fx = ox * step_x + step_x / 2 - 0x8000;
                    let (tx0, tx1, wx) = taps(fx, w);
                    let sample = |sx: i64, sy: i64| -> [u32; 4] {
                        let at = sy as usize * src_stride + sx as usize * 4;
                        match self.pixels.get(at..at + 4) {
                            Some(p) => [
                                u32::from(p[0]),
                                u32::from(p[1]),
                                u32::from(p[2]),
                                u32::from(p[3]),
                            ],
                            None => [0; 4],
                        }
                    };
                    let (a, b) = (sample(tx0, ty0), sample(tx1, ty0));
                    let (c, e) = (sample(tx0, ty1), sample(tx1, ty1));
                    let mut mixed = [0u8; 4];
                    for channel in 0..4 {
                        let top = a[channel] * (256 - wx) + b[channel] * wx;
                        let bottom = c[channel] * (256 - wx) + e[channel] * wx;
                        mixed[channel] = ((top * (256 - wy) + bottom * wy + 0x8000) >> 16) as u8;
                    }
                    over(d, mixed, g);
                }
            }
        }
    }
}

/// The two source rows (or columns) a fixed-point position `f` blends, and
/// the second's weight out of 256. Clamped to `0..len`.
fn taps(f: i64, len: i64) -> (i64, i64, u32) {
    let f = f.max(0);
    let low = (f >> 16).min(len - 1);
    let high = (low + 1).min(len - 1);
    let weight = if low == high {
        0
    } else {
        ((f >> 8) & 0xff) as u32
    };
    (low, high, weight)
}

/// `value / 255`, rounded (exact for `0 ..= 255 * 255`).
fn div255(value: u32) -> u32 {
    let value = value + 128;
    (value + (value >> 8)) >> 8
}

/// Source-over of the premultiplied `src` at global alpha `g` onto the
/// premultiplied pixel `dst`. An opaque source at full alpha is a copy.
fn over(dst: &mut [u8], src: [u8; 4], g: u32) {
    if g == 255 {
        if src[3] == 255 {
            dst.copy_from_slice(&src);
            return;
        }
        if src[3] == 0 {
            return;
        }
    }
    let scaled = |c: u8| {
        if g == 255 {
            u32::from(c)
        } else {
            div255(u32::from(c) * g)
        }
    };
    let inverse = 255 - scaled(src[3]);
    for (d, s) in dst.iter_mut().zip(src) {
        let blended = scaled(s) + div255(u32::from(*d) * inverse);
        *d = blended.min(255) as u8;
    }
}

#[cfg(test)]
mod tests;
