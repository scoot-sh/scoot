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
use std::cell::RefCell;

/// The most pixels a layer holds (8 MiB of `ARGB8888`): a card larger than
/// this is not worth moving and is drawn in place instead. The shell's
/// cards are a few hundred pixels across.
pub const MAX_LAYER_PIXELS: usize = 2 * 1024 * 1024;

/// A reusable off-screen premultiplied `ARGB8888` image.
#[derive(Debug)]
pub struct Layer {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    limit: usize,
    /// The vertically-blended source row the scaled path reuses: `width *
    /// 4` `u16` samples in `0..=65280` (`width` is fixed by [`Layer::begin`],
    /// so [`Layer::composite`] never grows it and allocates nothing).
    /// `RefCell` keeps [`Layer::composite`]'s `&self` shape at its call
    /// sites; the layer is only composited on one thread.
    scratch: RefCell<Vec<u16>>,
}

impl Default for Layer {
    fn default() -> Self {
        Self::new()
    }
}

impl Layer {
    /// An empty layer: holds no pixels until [`Layer::begin`].
    pub fn new() -> Self {
        Self::with_limit(MAX_LAYER_PIXELS)
    }

    /// An empty layer that holds up to `max_pixels` pixels instead of
    /// [`MAX_LAYER_PIXELS`]: for a surface that is moved whatever its size
    /// (a launcher panel at a large font on a HiDPI output), where the
    /// layer is no larger than the buffer the surface already needs.
    pub fn with_limit(max_pixels: usize) -> Self {
        Self {
            pixels: Vec::new(),
            width: 0,
            height: 0,
            limit: max_pixels,
            scratch: RefCell::new(Vec::new()),
        }
    }

    /// Reshapes the layer to `width` × `height` and clears it to
    /// transparent, keeping its allocation when it is large enough.
    /// Returns false (and leaves the layer empty) for a size that is zero
    /// or larger than the layer's limit ([`MAX_LAYER_PIXELS`] unless made
    /// by [`Layer::with_limit`]).
    pub fn begin(&mut self, width: u32, height: u32) -> bool {
        self.width = 0;
        self.height = 0;
        let pixels = (width as usize).saturating_mul(height as usize);
        if pixels == 0 || pixels > self.limit {
            return false;
        }
        self.pixels.clear();
        self.pixels.resize(pixels * 4, 0);
        // Pool the scaled path's scratch row for this width: `resize`
        // within capacity allocates nothing, so shrinking and regrowing to
        // a width the layer held before stays free.
        let mut scratch = self.scratch.borrow_mut();
        scratch.clear();
        scratch.resize(width as usize * 4, 0);
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

    /// The layer's pixels, `width * height * 4` bytes of premultiplied
    /// `ARGB8888` with rows packed: for a painter that takes a pixel
    /// slice and its size rather than a [`Canvas`]. Empty while the layer
    /// is empty.
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        let len = (self.width as usize) * (self.height as usize) * 4;
        self.pixels.get_mut(..len).unwrap_or_default()
    }

    /// The layer's pixels, read-only: [`Layer::pixels_mut`]'s view.
    pub fn pixels(&self) -> &[u8] {
        let len = (self.width as usize) * (self.height as usize) * 4;
        self.pixels.get(..len).unwrap_or_default()
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
        // The scaled path's per-row state, hoisted out of the row loop: one
        // division per axis for the whole composite (the old code divided
        // per row), and one pooled scratch row for the vertical blend.
        let (step_x, step_y) = if unscaled {
            (0, 0)
        } else {
            ((w << 16) / dw, (h << 16) / dh)
        };
        // Borrow the scratch row once for the whole scaled run: the
        // vertical blend writes it, the horizontal blend reads it.
        // `begin` sized it to `width * 4`, so this resizes nothing in
        // steady state and the composite allocates nothing.
        let mut scratch_guard = self.scratch.borrow_mut();
        let scratch_row: &mut [u16] = if unscaled {
            &mut []
        } else {
            let need = self.width as usize * 4;
            if scratch_guard.len() != need {
                scratch_guard.clear();
                scratch_guard.resize(need, 0);
            }
            &mut scratch_guard[..need]
        };
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
                for (d, s) in out
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(src.as_chunks::<4>().0)
                {
                    over(d, *s, g);
                }
            } else {
                // Bilinear in 16.16 fixed point: the source position of a
                // destination pixel center, minus half a source pixel.
                // Two passes with exactly the old integer arithmetic,
                // reordered: the two source rows blend vertically once per
                // destination row into the pooled scratch row (the weights
                // are the same for every pixel of the row), then each
                // destination pixel blends horizontally out of the scratch.
                // The old per-pixel value was
                //   ((a*(256-wx)+b*wx)*(256-wy)+(c*(256-wx)+e*wx)*wy+0x8000)>>16
                // per channel; the scratch holds
                //   v0 = a*(256-wy)+c*wy, v1 = b*(256-wy)+e*wy
                // and the horizontal pass computes
                //   (v0*(256-wx)+v1*wx+0x8000)>>16,
                // the same integer (addition and multiplication reorder
                // exactly). Both passes run over contiguous slices, which
                // is what vectorizes; the per-column taps stay computed
                // on the fly (one multiply each, no division, no table).
                let fy = (row - top) * step_y + step_y / 2 - 0x8000;
                let (ty0, ty1, wy) = taps(fy, h);
                let source_row = |ty: i64| {
                    self.pixels
                        .get(ty as usize * src_stride..)
                        .and_then(|rest| rest.get(..src_stride))
                };
                let (Some(upper), Some(lower)) = (source_row(ty0), source_row(ty1)) else {
                    continue;
                };
                // Vertical pass: one blended row of `w` pixels in `u16`
                // (`0..=65280`, since the weights sum to 256). Contiguous,
                // so the compiler vectorizes it.
                let inv_y = 256 - wy;
                let mut row_has_ink = false;
                // Fast path when the row's lower weight is 0: the
                // scratch is the upper row widened, with no arithmetic.
                // `wy == 0` does not imply `ty0 == ty1` (`taps()` can
                // return weight 0 with distinct taps within 1/256 of a
                // pixel boundary); the fast path is still correct because
                // the lower row's weight is 0.
                if wy == 0 {
                    for (v, b) in scratch_row.iter_mut().zip(upper.iter()) {
                        let wide = u16::from(*b);
                        *v = wide * 256;
                        row_has_ink |= wide != 0;
                    }
                } else {
                    for (v, pair) in scratch_row.iter_mut().zip(upper.iter().zip(lower.iter())) {
                        let (u, l) = (u32::from(*pair.0), u32::from(*pair.1));
                        let blended = u * inv_y + l * wy;
                        *v = blended as u16;
                        row_has_ink |= blended != 0;
                    }
                }
                if !row_has_ink {
                    // Both source rows are clear: every destination pixel
                    // would blend to clear, which `over` skips.
                    continue;
                }
                for (offset, d) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let ox = x0 - left + offset as i64;
                    let fx = ox * step_x + step_x / 2 - 0x8000;
                    let (tx0, tx1, wx) = taps(fx, w);
                    let (at0, at1) = (tx0 as usize * 4, tx1 as usize * 4);
                    let (Some(v0), Some(v1)) =
                        (scratch_row.get(at0..at0 + 4), scratch_row.get(at1..at1 + 4))
                    else {
                        continue;
                    };
                    // Four clear source pixels blend to a clear one, which
                    // `over` leaves the destination alone for: skip the
                    // arithmetic (a shadow's room is mostly this).
                    if v0.iter().chain(v1.iter()).all(|s| *s == 0) {
                        continue;
                    }
                    let mut mixed = [0u8; 4];
                    let inv_x = 256 - wx;
                    for channel in 0..4 {
                        let blended = u32::from(v0[channel]) * inv_x + u32::from(v1[channel]) * wx;
                        mixed[channel] = ((blended + 0x8000) >> 16) as u8;
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
