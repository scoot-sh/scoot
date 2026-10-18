//! The bar's own fill: a [`Vector`] as anti-aliased coverage, analytically,
//! with no supersampling.
//!
//! The method is the signed-area accumulation `font-rs` and `ab_glyph`
//! use for glyphs. Each edge adds, into a buffer one float per pixel, the
//! area it uncovers to its right within each pixel it crosses; a running
//! sum along each row then gives every pixel's coverage, exact for a
//! simple shape, in one pass over the edges and one over the pixels. A
//! curve is flattened first into at most [`MAX_FLATTEN`] lines, chosen so
//! the polyline stays within a fortieth of a pixel of it.
//!
//! Fill rule: coverage is the accumulated winding clamped to 0 to 1, which
//! is SVG's default `nonzero` for icons as drawn (a hole wound the other
//! way cancels; overlapping same-direction shapes saturate). Two subpaths
//! that overlap with opposite winding cancel, where `nonzero` would too,
//! and a partly-covered pixel where two edges of opposite winding meet
//! counts the difference, a sub-pixel error `nonzero` would not make.
//!
//! The path is fitted into the square by SVG's `xMidYMid meet`: the
//! viewbox scaled uniformly to fit, centered. Everything is clamped to the
//! buffer before it indexes it, and every access is checked: an edge far
//! outside the square (a path past its viewbox) costs nothing and cannot
//! write outside the buffer.

use super::path::{Point, Seg, Vector};

#[cfg(test)]
mod tests;

/// The most lines one curve becomes.
pub const MAX_FLATTEN: usize = 48;

/// The accumulation buffer, kept between fills so a fill allocates only
/// when a larger square than any before is asked for.
#[derive(Debug, Default)]
pub struct Rasterizer {
    acc: Vec<f32>,
    side: usize,
    /// Row length: two columns past the square, which an edge clamped to
    /// the right edge writes into.
    stride: usize,
}

impl Rasterizer {
    /// Fills `vector` into `out`, `side × side` coverage bytes, row by
    /// row. `out` shorter than that is left untouched.
    pub fn fill(&mut self, vector: &Vector, side: u32, out: &mut [u8]) {
        let side = side as usize;
        let Some(len) = side.checked_mul(side) else {
            return;
        };
        let Some(out) = out.get_mut(..len) else {
            return;
        };
        out.fill(0);
        if side == 0 {
            return;
        }
        self.side = side;
        self.stride = side + 2;
        self.acc.clear();
        self.acc.resize(self.stride * side, 0.0);

        let view = vector.view();
        let side_f = side as f32;
        let scale = (side_f / view.width).min(side_f / view.height);
        let offset = (
            (side_f - view.width * scale) / 2.0 - view.x * scale,
            (side_f - view.height * scale) / 2.0 - view.y * scale,
        );
        let map = |(x, y): Point| (x * scale + offset.0, y * scale + offset.1);

        let (mut cur, mut start) = ((0.0, 0.0), (0.0, 0.0));
        for seg in vector.segs() {
            match *seg {
                Seg::Move(p) => {
                    // A fill closes the subpath it leaves.
                    self.line(cur, start);
                    cur = map(p);
                    start = cur;
                }
                Seg::Line(p) => {
                    let to = map(p);
                    self.line(cur, to);
                    cur = to;
                }
                Seg::Cubic(a, b, p) => {
                    let to = map(p);
                    self.cubic(cur, map(a), map(b), to);
                    cur = to;
                }
                Seg::Close => {
                    self.line(cur, start);
                    cur = start;
                }
            }
        }
        self.line(cur, start);

        for (row, out) in self
            .acc
            .chunks_exact(self.stride)
            .zip(out.chunks_exact_mut(side))
        {
            let mut sum = 0.0f32;
            for (&a, pixel) in row.iter().zip(out) {
                sum += a;
                *pixel = (sum.abs().min(1.0) * 255.0 + 0.5) as u8;
            }
        }
    }

    /// A cubic as lines: as many as keep it within 0.025 px (the deviation
    /// of a uniform split is at most `6 × M ÷ (8 n²)` for `M` the larger
    /// second difference of the control points, so `n = √(30 M)`), at least
    /// one and at most [`MAX_FLATTEN`]. A polygon inscribed in a curve
    /// always falls short of it, which for a filled circle is an area
    /// error of about a third of a percent at this tolerance (a tenth of a
    /// pixel would be a full one, visibly small icons).
    fn cubic(&mut self, p0: Point, p1: Point, p2: Point, p3: Point) {
        let second =
            |a: Point, b: Point, c: Point| (a.0 - 2.0 * b.0 + c.0).hypot(a.1 - 2.0 * b.1 + c.1);
        let m = second(p0, p1, p2).max(second(p1, p2, p3));
        let n = (30.0 * m).sqrt().ceil();
        // NaN (a curve off to infinity) is one line, which `line` drops.
        let n = if n.is_finite() {
            (n as usize).clamp(1, MAX_FLATTEN)
        } else {
            1
        };
        let mut previous = p0;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            let next = if i == n {
                p3
            } else {
                (
                    a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
                    a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
                )
            };
            self.line(previous, next);
            previous = next;
        }
    }

    fn add(&mut self, y: usize, x: usize, v: f32) {
        if let Some(cell) = self.acc.get_mut(y * self.stride + x) {
            *cell += v;
        }
    }

    /// One edge's area, into each row it crosses.
    fn line(&mut self, a: Point, b: Point) {
        if !(a.0.is_finite() && a.1.is_finite() && b.0.is_finite() && b.1.is_finite()) {
            return;
        }
        let (dir, p0, p1) = if a.1 < b.1 {
            (1.0f32, a, b)
        } else {
            (-1.0, b, a)
        };
        let height = self.side as f32;
        let width = self.side as f32;
        // Horizontal (no area), or wholly above or below the square.
        if p1.1 - p0.1 < 1.0e-4 || p1.1 <= 0.0 || p0.1 >= height {
            return;
        }
        let dxdy = (p1.0 - p0.0) / (p1.1 - p0.1);
        if !dxdy.is_finite() {
            return;
        }
        // Clipped to the square's rows, with x carried along the edge.
        let (mut x, top) = if p0.1 < 0.0 {
            (p0.0 - p0.1 * dxdy, 0.0)
        } else {
            (p0.0, p0.1)
        };
        let bottom = p1.1.min(height);
        let first = top as usize;
        let last = (bottom.ceil() as usize).min(self.side);
        for y in first..last {
            let dy = ((y + 1) as f32).min(bottom) - (y as f32).max(top);
            let next = x + dxdy * dy;
            let d = dy * dir;
            // Left of the square counts as its left edge, right of it as
            // its right: the winding they add still reaches the pixels to
            // their right (or none, past the edge).
            let (lo, hi) = if x < next { (x, next) } else { (next, x) };
            let (lo, hi) = (lo.clamp(0.0, width), hi.clamp(0.0, width));
            self.span(y, lo, hi, d);
            x = next;
        }
    }

    /// The area of an edge crossing row `y` from `lo` to `hi` (`lo <= hi`,
    /// both in `0..=side`), carrying `d` of winding.
    fn span(&mut self, y: usize, lo: f32, hi: f32, d: f32) {
        let lo_floor = lo.floor();
        let lo_i = lo_floor as usize;
        let hi_ceil = hi.ceil();
        let hi_i = hi_ceil as usize;
        if hi_i <= lo_i + 1 {
            // Within one pixel.
            let mid = 0.5 * (lo + hi) - lo_floor;
            self.add(y, lo_i, d - d * mid);
            self.add(y, lo_i + 1, d * mid);
            return;
        }
        let s = 1.0 / (hi - lo);
        let lo_f = lo - lo_floor;
        let a0 = 0.5 * s * (1.0 - lo_f) * (1.0 - lo_f);
        let hi_f = hi - hi_ceil + 1.0;
        let am = 0.5 * s * hi_f * hi_f;
        self.add(y, lo_i, d * a0);
        if hi_i == lo_i + 2 {
            self.add(y, lo_i + 1, d * (1.0 - a0 - am));
        } else {
            let a1 = s * (1.5 - lo_f);
            self.add(y, lo_i + 1, d * (a1 - a0));
            for x in lo_i + 2..hi_i - 1 {
                self.add(y, x, d * s);
            }
            let a2 = a1 + (hi_i - lo_i - 3) as f32 * s;
            self.add(y, hi_i - 1, d * (1.0 - a2 - am));
        }
        self.add(y, hi_i, d * am);
    }
}
