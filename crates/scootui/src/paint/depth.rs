//! A soft drop shadow under a rounded rectangle, from tables built once.
//!
//! The shadow of a rounded rectangle is a function of one distance: how far
//! a pixel sits inside the (offset) shape's edge, `u` from the nearest
//! vertical edge and `v` from the nearest horizontal one. Along the straight
//! runs (`max(u, v) >= radius`) only the nearer edge matters, so one 1-D
//! falloff table serves every size of card; only inside a corner square
//! does the pair matter, and that is a `(radius + blur)²` table per corner
//! radius. So the cache is keyed by `(radius, blur)`, not by card size, and
//! a stack of cards of any heights shares it. The color is applied at
//! blend time, so one cache serves a black shadow and every tint of a glow.
//!
//! The edge profile is a smoothstep across `2 × blur` pixels centered on
//! the edge, a close fit to a Gaussian blur of `sigma = blur / 2` (half
//! the strength at the edge itself) at the cost of a table lookup per
//! pixel. Painting writes only the pixels the shadow reaches and skips the
//! pixels the card itself covers outright, so a translucent card does not
//! show its own shadow through its body. Nothing here allocates once the
//! tables are built; [`Shadow::prepare`] builds them, outside any repaint.
//!
//! Glow is not a separate primitive: it is this shadow with zero offset
//! and a tint.

use super::Canvas;
use crate::color::Color;
use crate::region::{Rect, effective_radius};

/// The largest blur radius, in pixels: past it a table is bigger than any
/// shadow could want and the surface padding would be absurd. The deepest
/// style's 28 px blur at a 3x output is 84 of them.
pub const MAX_BLUR: u32 = 128;
/// The largest offset on either axis, in pixels.
pub const MAX_OFFSET: i32 = 64;

/// What a shadow looks like, apart from its color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowSpec {
    /// How far right the shadow sits from the shape, in pixels (negative:
    /// left). Clamped to [`MAX_OFFSET`].
    pub offset_x: i32,
    /// How far down it sits (negative: up). Clamped to [`MAX_OFFSET`].
    pub offset_y: i32,
    /// The blur radius, in pixels: the shadow reaches this far past its
    /// own edge. 0 is a hard, anti-aliased edge. Clamped to [`MAX_BLUR`].
    pub blur: u32,
    /// The shadow's strength at full coverage, 0 to 255.
    pub alpha: u8,
}

/// Room a shadow needs around its shape, per side, in pixels: how far it
/// reaches past the shape's bounding box. A surface that draws a shadow is
/// this much larger than the shapes it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pad {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
}

impl Pad {
    /// No room: the surface is exactly its content.
    pub const ZERO: Self = Self {
        top: 0,
        right: 0,
        bottom: 0,
        left: 0,
    };

    /// Whether every side is 0.
    pub fn is_zero(self) -> bool {
        self == Self::ZERO
    }
}

/// The alpha table of one corner radius: `side × side`, `side = radius +
/// blur`, indexed `(u + blur) * side + (v + blur)` for `u, v` in `-blur ..
/// radius`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Corner {
    radius: u32,
    alpha: Vec<u8>,
}

/// A shadow's tables: the edge falloff and one corner table per radius in
/// use. Build with [`Shadow::new`], size the corners with
/// [`Shadow::prepare`], paint with [`Shadow::paint`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shadow {
    spec: ShadowSpec,
    /// `edge[k + blur]`: the alpha `k` pixels inside the edge, `k` from
    /// `-blur` to `blur`; deeper is the last entry (full strength).
    edge: Vec<u8>,
    corners: Vec<Corner>,
}

impl Shadow {
    /// The tables for `spec` (blur and offsets clamped). No corner table
    /// yet: [`Shadow::prepare`] adds the radii a stack uses.
    pub fn new(spec: ShadowSpec) -> Self {
        let spec = ShadowSpec {
            offset_x: spec.offset_x.clamp(-MAX_OFFSET, MAX_OFFSET),
            offset_y: spec.offset_y.clamp(-MAX_OFFSET, MAX_OFFSET),
            blur: spec.blur.min(MAX_BLUR),
            alpha: spec.alpha,
        };
        let blur = i64::from(spec.blur);
        let edge = (-blur..=blur)
            .map(|k| strength(&spec, -(k as f64 + 0.5)))
            .collect();
        Self {
            spec,
            edge,
            corners: Vec::new(),
        }
    }

    /// The room this shadow needs around the shapes it sits under.
    pub fn pad(&self) -> Pad {
        let (blur, ox, oy) = (
            i64::from(self.spec.blur),
            i64::from(self.spec.offset_x),
            i64::from(self.spec.offset_y),
        );
        let side = |value: i64| value.max(0) as u32;
        Pad {
            top: side(blur - oy),
            right: side(blur + ox),
            bottom: side(blur + oy),
            left: side(blur - ox),
        }
    }

    /// Builds the corner tables for shapes of the given `(width, height)`
    /// drawn with `radius`, and drops the tables nothing uses any more.
    /// Allocates when a radius is new; call it when the content changes,
    /// never per repaint.
    pub fn prepare(&mut self, radius: u32, sizes: impl IntoIterator<Item = (u32, u32)>) {
        let mut wanted: Vec<u32> = sizes
            .into_iter()
            .map(|(width, height)| effective_radius(radius, width, height))
            .collect();
        wanted.sort_unstable();
        wanted.dedup();
        self.corners
            .retain(|corner| wanted.contains(&corner.radius));
        for radius in wanted {
            if self.corners.iter().all(|corner| corner.radius != radius) {
                let corner = self.build_corner(radius);
                self.corners.push(corner);
            }
        }
    }

    fn build_corner(&self, radius: u32) -> Corner {
        let blur = i64::from(self.spec.blur);
        let r = f64::from(radius);
        let side = (radius + self.spec.blur) as usize;
        let mut alpha = Vec::with_capacity(side * side);
        for u in -blur..i64::from(radius) {
            for v in -blur..i64::from(radius) {
                // Pixel centers, inward from the two edges; the corner
                // circle's center sits `radius` in from both.
                let (px, py) = (u as f64 + 0.5, v as f64 + 0.5);
                // `sqrt` of the squares, not `hypot`: sqrt is correctly
                // rounded everywhere, so the table (and the goldens that
                // fingerprint it) is the same on every platform.
                let distance = ((r - px) * (r - px) + (r - py) * (r - py)).sqrt() - r;
                alpha.push(strength(&self.spec, distance));
            }
        }
        Corner { radius, alpha }
    }

    /// Paints the shadow of `card` (rounded by `radius`, cut back to what
    /// the card holds) in `color` into `canvas`, source-over on the
    /// premultiplied buffer, clipped to it. The pixels the card covers
    /// outright are left alone. The corner table for the card's radius
    /// must have been [`prepare`](Shadow::prepare)d; without it the corners
    /// are drawn square, never a panic. Allocates nothing.
    pub fn paint(&self, canvas: &mut Canvas<'_>, card: Rect, radius: u32, color: Color) {
        if card.width == 0 || card.height == 0 || self.spec.alpha == 0 {
            return;
        }
        let r = effective_radius(radius, card.width, card.height);
        let corner = self.corners.iter().find(|corner| corner.radius == r);
        let blur = i64::from(self.spec.blur);
        let sx0 = i64::from(card.x) + i64::from(self.spec.offset_x);
        let sy0 = i64::from(card.y) + i64::from(self.spec.offset_y);
        let (sx1, sy1) = (sx0 + i64::from(card.width), sy0 + i64::from(card.height));
        let (width, height) = (i64::from(canvas.width), i64::from(canvas.height));
        let (x_from, x_to) = ((sx0 - blur).max(0), (sx1 + blur).min(width));
        let (y_from, y_to) = ((sy0 - blur).max(0), (sy1 + blur).min(height));
        if x_from >= x_to || y_from >= y_to {
            return;
        }
        let shape = Shape { sx0, sx1, r };
        let radius = i64::from(r);
        for y in y_from..y_to {
            let v = (y - sy0).min(sy1 - 1 - y);
            let (hole_from, hole_to) = covered_run(card, r, y);
            let row = y as usize * canvas.width as usize * 4;
            // Between the corner squares a row above the straight sides
            // (`v < radius`: the rows beside the corners, and every row
            // outside the shape) is one alpha along its whole length: the
            // falloff of its distance from the horizontal edge.
            let middle = (v < radius && sx0 + radius < sx1 - radius)
                .then(|| (sx0 + radius, sx1 - radius, self.edge_at(v)));
            for (from, to) in [(x_from, x_to.min(hole_from)), (x_from.max(hole_to), x_to)] {
                if from >= to {
                    continue;
                }
                let (steady_from, steady_to, steady) = match middle {
                    Some((a, b, alpha)) if from.max(a) < to.min(b) => {
                        (from.max(a), to.min(b), alpha)
                    }
                    _ => (to, to, 0),
                };
                for (start, end, constant) in [
                    (from, steady_from, None),
                    (steady_from, steady_to, Some(steady)),
                    (steady_to, to, None),
                ] {
                    if start >= end {
                        continue;
                    }
                    let Some(run) = canvas
                        .pixels
                        .get_mut(row + start as usize * 4..row + end as usize * 4)
                    else {
                        continue;
                    };
                    for (index, pixel) in run.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                        let alpha = match constant {
                            Some(alpha) => alpha,
                            None => {
                                let u = shape.inward(start + index as i64);
                                self.alpha_at(corner, shape.r, u, v)
                            }
                        };
                        if alpha != 0 {
                            over(pixel, color, alpha);
                        }
                    }
                }
            }
        }
    }

    /// The shadow's alpha `u` pixels in from the nearest vertical edge and
    /// `v` from the nearest horizontal one (negative: outside).
    fn alpha_at(&self, corner: Option<&Corner>, radius: u32, u: i64, v: i64) -> u8 {
        let blur = i64::from(self.spec.blur);
        let r = i64::from(radius);
        if u.max(v) < r
            && let Some(corner) = corner
        {
            if u < -blur || v < -blur {
                return 0;
            }
            let side = (r + blur) as usize;
            return corner
                .alpha
                .get((u + blur) as usize * side + (v + blur) as usize)
                .copied()
                .unwrap_or(0);
        }
        self.edge_at(u.min(v))
    }

    fn edge_at(&self, k: i64) -> u8 {
        let blur = i64::from(self.spec.blur);
        if k < -blur {
            0
        } else {
            self.edge
                .get((k + blur) as usize)
                .or_else(|| self.edge.last())
                .copied()
                .unwrap_or(0)
        }
    }
}

/// The shadow shape's horizontal extent and corner radius.
struct Shape {
    sx0: i64,
    sx1: i64,
    r: u32,
}

impl Shape {
    /// How far inside the nearer vertical edge column `x` is.
    fn inward(&self, x: i64) -> i64 {
        (x - self.sx0).min(self.sx1 - 1 - x)
    }
}

/// The shadow strength (0 to `spec.alpha`) at signed distance `d` from the
/// shape's edge (negative inside), pixel centers.
fn strength(spec: &ShadowSpec, d: f64) -> u8 {
    let blur = f64::from(spec.blur);
    let fraction = if spec.blur == 0 {
        (0.5 - d).clamp(0.0, 1.0)
    } else {
        let x = ((d + blur) / (2.0 * blur)).clamp(0.0, 1.0);
        1.0 - x * x * (3.0 - 2.0 * x)
    };
    (fraction * f64::from(spec.alpha)).round() as u8
}

/// The columns `from .. to` of row `y` that `card` covers outright (full
/// coverage: its interior and the straight parts of its corner rows); an
/// empty run when the row misses the card. A pixel is covered outright when
/// its center is within `radius - 0.5` of the corner circle's center, the
/// same test [`Canvas::fill_rounded_gradient`] rounds its coverage with.
fn covered_run(card: Rect, radius: u32, y: i64) -> (i64, i64) {
    let top = i64::from(card.y);
    let bottom = top + i64::from(card.height);
    if y < top || y >= bottom {
        return (0, 0);
    }
    let (left, right) = (i64::from(card.x), i64::from(card.x) + i64::from(card.width));
    let into = (y - top).min(bottom - 1 - y);
    let r = f64::from(radius);
    if into >= i64::from(radius) {
        return (left, right);
    }
    // The row center's distance from the corner circle's center, vertically.
    let dy = r - (into as f64 + 0.5);
    let reach = (r - 0.5) * (r - 0.5) - dy * dy;
    let inset = (r - 0.5 - reach.max(0.0).sqrt()).ceil().clamp(0.0, r) as i64;
    (left + inset, right - inset)
}

/// Composites `color` at `alpha` over the premultiplied `b, g, r, a` pixel:
/// the source-over `(c × a + dst × (255 - a)) / 255`, rounded, on every
/// channel, the opaque color's alpha being 255. The arithmetic of
/// [`Canvas::blend`].
fn over(pixel: &mut [u8; 4], color: Color, alpha: u8) {
    let a = u32::from(alpha);
    let mix =
        |src: u8, dst: u8| ((u32::from(src) * a + u32::from(dst) * (255 - a) + 127) / 255) as u8;
    let [blue, green, red, coverage] = pixel;
    *blue = mix(color.b, *blue);
    *green = mix(color.g, *green);
    *red = mix(color.r, *red);
    *coverage = mix(0xff, *coverage);
}

#[cfg(test)]
mod tests;
