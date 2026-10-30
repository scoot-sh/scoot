//! A rounded rectangle filled with a vertical gradient, composited over
//! what is already in the buffer.
//!
//! Coverage is analytic per pixel (a pixel's center within half a pixel of
//! the corner circle), and every pixel goes through [`Canvas::blend`], so
//! the fill reads correctly over any destination: an inner fill over its
//! border ring, a translucent card over a shadow, text over the fill. It
//! allocates nothing and needs no table. A solid fill is the gradient with
//! one stop twice, and writes the bytes the plain fill always wrote.

use super::{Canvas, Span};
use crate::color::Color;
use crate::region::{Rect, effective_radius};

impl Canvas<'_> {
    /// Fills `rect` (clipped to the canvas) with rounded corners of
    /// `radius` (cut back to what the rectangle holds) and a vertical
    /// gradient from `top` (the first row) to `bottom` (the last), at
    /// `alpha` (255 opaque). `top == bottom` is a solid fill. Premultiplied
    /// `ARGB8888` only: on `XRGB8888` the blend would corrupt the pad byte.
    ///
    /// Only the corner squares compute coverage; everything between them
    /// is one color at one strength, written as a run (an opaque run is a
    /// plain store, which is what the blend of an opaque color is, byte
    /// for byte).
    pub fn fill_rounded_gradient(
        &mut self,
        rect: Rect,
        radius: u32,
        top: Color,
        bottom: Color,
        alpha: u8,
    ) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        let radius = effective_radius(radius, rect.width, rect.height);
        let rows = rect.height;
        let y_end = rect.y.saturating_add(rows).min(self.height());
        let x_end = rect.x.saturating_add(rect.width).min(self.width());
        if rect.x >= x_end {
            return;
        }
        let corner_width = u64::from(radius);
        let right_corner = u64::from(rect.x) + u64::from(rect.width) - corner_width;
        for y in rect.y..y_end {
            let color = row_color(top, bottom, y - rect.y, rows);
            let from_top = y - rect.y;
            let in_corner_band = radius > 0 && (from_top < radius || from_top >= rows - radius);
            // The straight run of this row: the whole row between the
            // corners, or all of it outside the corner bands.
            let (mut run_from, mut run_to) = (rect.x, x_end);
            if in_corner_band {
                // Each corner square of the row is three stretches: clear
                // (coverage 0), the anti-aliased few, and whole (a run).
                // Coverage grows toward the shape inward, so the two
                // boundaries are found by search instead of evaluating
                // every pixel: a pill's 36 px corners cost a few
                // evaluations a row, not 36.
                let left_end = rect.x.saturating_add(radius).min(x_end);
                let covered = |x: u32| rounded_coverage(x, y, rect, radius);
                let seen = first_where(rect.x, left_end, |x| covered(x) > 0);
                let whole = first_where(seen, left_end, |x| covered(x) == 255);
                for x in seen..whole {
                    self.blend_covered(x, y, rect, radius, alpha, color);
                }
                self.blend_run(whole, left_end, y, alpha, color);
                run_from = left_end;
                let right_start = u32::try_from(right_corner).unwrap_or(u32::MAX);
                let first = right_start.max(run_from);
                let whole_end = first_where(first, x_end, |x| covered(x) < 255);
                let gone = first_where(whole_end, x_end, |x| covered(x) == 0);
                self.blend_run(first, whole_end, y, alpha, color);
                for x in whole_end..gone {
                    self.blend_covered(x, y, rect, radius, alpha, color);
                }
                run_to = right_start.min(x_end);
            }
            if run_from < run_to {
                self.blend_run(run_from, run_to, y, alpha, color);
            }
        }
    }

    /// Paints the leftmost `width` columns of the rounded rectangle `rect`
    /// (clipped to the canvas, corners of `radius` cut back to what the
    /// rectangle holds) in `color`, opaque, shaped by the same coverage as
    /// [`Canvas::fill_rounded_gradient`]: an accent bar down a card's
    /// leading edge that follows the card's corners instead of spilling
    /// past them. A `width` past the rectangle's is its whole width. The
    /// straight rows are plain stores; only the corner rows compute
    /// coverage. Premultiplied `ARGB8888` only. Allocates nothing.
    pub fn fill_rounded_edge(&mut self, rect: Rect, radius: u32, width: u32, color: Color) {
        if rect.width == 0 || rect.height == 0 || width == 0 {
            return;
        }
        let radius = effective_radius(radius, rect.width, rect.height);
        let x_end = rect
            .x
            .saturating_add(width.min(rect.width))
            .min(self.width());
        let y_end = rect.y.saturating_add(rect.height).min(self.height());
        let clip = Span {
            x: 0,
            width: self.width(),
        };
        for y in rect.y..y_end {
            let from_top = y - rect.y;
            let from_bottom = rect.height - 1 - from_top;
            if radius == 0 || (from_top >= radius && from_bottom >= radius) {
                self.blend_run(rect.x, x_end, y, 255, color);
                continue;
            }
            for x in rect.x..x_end {
                match rounded_coverage(x, y, rect, radius) {
                    0 => {}
                    255 => self.blend_run(x, x + 1, y, 255, color),
                    coverage => self.blend(i64::from(x), i64::from(y), coverage, color, clip),
                }
            }
        }
    }

    /// One corner pixel: the fill's alpha times the circle's coverage,
    /// through one blend.
    fn blend_covered(&mut self, x: u32, y: u32, rect: Rect, radius: u32, alpha: u8, color: Color) {
        let coverage = rounded_coverage(x, y, rect, radius);
        if coverage == 0 {
            return;
        }
        let blended = (u32::from(alpha) * u32::from(coverage) + 127) / 255;
        let clip = Span {
            x: 0,
            width: self.width(),
        };
        self.blend(
            i64::from(x),
            i64::from(y),
            blended.min(255) as u8,
            color,
            clip,
        );
    }

    /// `color` at `alpha` over columns `from .. to` of row `y`, both
    /// inside the canvas: the arithmetic of [`Canvas::blend`] on a whole
    /// slice, and a plain store when `alpha` is 255.
    fn blend_run(&mut self, from: u32, to: u32, y: u32, alpha: u8, color: Color) {
        if alpha == 0 {
            return;
        }
        let row = y as usize * self.width() as usize * 4;
        let Some(run) = self
            .pixels
            .get_mut(row + from as usize * 4..row + to as usize * 4)
        else {
            return;
        };
        if alpha == 255 {
            let opaque = [color.b, color.g, color.r, 0xff];
            for pixel in run.as_chunks_mut::<4>().0 {
                *pixel = opaque;
            }
            return;
        }
        let a = u32::from(alpha);
        let mix = |over: u8, under: u8| {
            ((u32::from(over) * a + u32::from(under) * (255 - a) + 127) / 255) as u8
        };
        for [blue, green, red, pad] in run.as_chunks_mut::<4>().0 {
            *blue = mix(color.b, *blue);
            *green = mix(color.g, *green);
            *red = mix(color.r, *red);
            *pad = mix(0xff, *pad);
        }
    }
}

/// The first `x` in `from..to` where `test` holds, or `to` when none does,
/// for a `test` that is false and then true across the range (a corner's
/// coverage is monotone along a row): a binary search.
fn first_where(from: u32, to: u32, test: impl Fn(u32) -> bool) -> u32 {
    let (mut low, mut high) = (from, to.max(from));
    while low < high {
        let mid = low + (high - low) / 2;
        if test(mid) {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    low
}

/// The gradient's color at row `row` of `rows`: sampled at the row's
/// center, so a two-row fill is not the stops themselves and a one-row
/// fill is their midpoint. Equal stops short-circuit to the stop.
pub fn row_color(top: Color, bottom: Color, row: u32, rows: u32) -> Color {
    if top == bottom {
        return top;
    }
    let toward = (u64::from(row) * 2 + 1) * 255 / (u64::from(rows) * 2);
    top.mix(bottom, toward.min(255) as u8)
}

/// The rounded-rectangle coverage of the pixel at `(x, y)` (0 to 255):
/// 255 in the straight runs, the circle's analytic coverage in the
/// corners (a pixel's center within half a pixel of the circle, as the
/// bar's own corner tables read it). `radius` is already
/// [`effective_radius`].
pub(super) fn rounded_coverage(x: u32, y: u32, rect: Rect, radius: u32) -> u8 {
    if radius == 0 {
        return 255;
    }
    // Widened: a rectangle at the edge of `u32` must not overflow.
    let (left, top) = (i64::from(rect.x), i64::from(rect.y));
    let (right, bottom) = (left + i64::from(rect.width), top + i64::from(rect.height));
    let (x, y, edge) = (i64::from(x), i64::from(y), i64::from(radius));
    let r = f64::from(radius);
    // Distances to the near corner center, when inside a corner square.
    let cx = if x < left + edge {
        (left + edge) as f64 - (x as f64 + 0.5)
    } else if x >= right - edge {
        (x as f64 + 0.5) - (right - edge) as f64
    } else {
        return 255;
    };
    let cy = if y < top + edge {
        (top + edge) as f64 - (y as f64 + 0.5)
    } else if y >= bottom - edge {
        (y as f64 + 0.5) - (bottom - edge) as f64
    } else {
        return 255;
    };
    let distance = (cx * cx + cy * cy).sqrt();
    ((r + 0.5 - distance).clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests;
