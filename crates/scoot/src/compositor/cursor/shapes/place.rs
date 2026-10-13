//! Where a Mocu cursor lands in a `size x size` bitmap.
//!
//! Mocu is drawn on a 24-unit grid, and at 24px scoot places it exactly
//! where Mocu's own build does: one unit per pixel, the hotspot on Mocu's
//! hotspot. Other sizes scale the art by `size / 24` around the hotspot (the
//! 16px default downscales Mocu's 24px art), shifted by whole pixels only
//! if the stroke or shadow would otherwise leave the bitmap, and shrunk
//! only if a shift cannot fit it.
//!
//! # Grid fitting
//!
//! Mocu's art is pixel-exact at 24px, where one unit is one pixel. At 16px
//! a unit is two thirds of a pixel, so a plain scale leaves every straight
//! edge straddling two pixels and the one-unit features (the I-beam's stem,
//! the gaps between the hand's fingers, the crosshair's hairlines) as grey
//! smears. So each axis is fitted to the pixel grid the way a font hinter
//! fits stems: the coordinates of the art's horizontal and vertical edges
//! (and, below 24px, its extremes) are *knots*, each knot is rounded to a whole pixel
//! (ties away from the hotspot, so symmetric art stays symmetric), knots a
//! unit or more apart stay at least a pixel apart, and everything between
//! them -- diagonals, arcs -- maps piecewise-linearly. Where a unit is a
//! whole number of pixels (24px, 48px) the fit changes none of Mocu's
//! whole-unit art; elsewhere it moves an edge by at most half a pixel.

use super::mocu::Art;
use super::svgpath::Subpath;
use smithay::utils::{Logical, Point};

/// The rim (Mocu's stroke, which is centred on the outline, so half its
/// width shows outside the fill) in whole pixels at `size`: Mocu's own
/// width scaled, never under one pixel -- Mocu's two-thirds-of-a-pixel rim
/// at 16px would be a grey smear rather than an edge.
pub(crate) fn rim(stroke: f64, size: i32) -> f64 {
    (stroke / 2.0 * f64::from(size) / 24.0).round().max(1.0)
}

/// Mocu's drop-shadow offset in pixels: one grid unit, down and right.
pub(crate) fn shadow_offset(size: i32) -> f64 {
    f64::from(size) / 24.0
}

/// One axis of a [`Placement`]: grid-unit knots and the pixel each lands
/// on, ascending in both.
#[derive(Clone, Debug)]
struct Axis {
    knots: Vec<(f64, f64)>,
    /// Pixels per unit, outside the knots.
    k: f64,
}

impl Axis {
    fn map(&self, u: f64) -> f64 {
        let ks = &self.knots;
        let Some(&(u0, p0)) = ks.first() else {
            return u * self.k;
        };
        if u <= u0 {
            return p0 - (u0 - u) * self.k;
        }
        for w in ks.windows(2) {
            let ((ua, pa), (ub, pb)) = (w[0], w[1]);
            if u <= ub {
                return pa + (u - ua) / (ub - ua) * (pb - pa);
            }
        }
        let &(un, pn) = ks.last().expect("non-empty");
        pn + (u - un) * self.k
    }

    fn shift(&mut self, by: f64) {
        for kn in &mut self.knots {
            kn.1 += by;
        }
    }
}

/// One cursor's mapping from Mocu's 24-grid into bitmap pixels.
#[derive(Clone, Debug)]
pub(crate) struct Placement {
    x: Axis,
    y: Axis,
    hot: (f64, f64),
}

impl Placement {
    /// A grid point in bitmap pixels.
    pub(crate) fn px(&self, x: f64, y: f64) -> (f64, f64) {
        (self.x.map(x), self.y.map(y))
    }

    /// The hotspot pixel: the one Mocu's hotspot lands in.
    pub(crate) fn hotspot(&self) -> Point<i32, Logical> {
        let (x, y) = self.px(self.hot.0, self.hot.1);
        (x.floor() as i32, y.floor() as i32).into()
    }

    /// Outlines in bitmap pixels.
    pub(crate) fn map(&self, subs: &[Subpath]) -> Vec<Subpath> {
        subs.iter()
            .map(|s| s.iter().map(|&(x, y)| self.px(x, y)).collect())
            .collect()
    }
}

/// The grid-unit knots of one axis: every coordinate where the art has an
/// edge along the *other* axis (`x` of vertical edges for the x axis), plus,
/// when downscaling, the art's two extremes (so its overall extent lands on
/// whole pixels too). Sorted, deduplicated.
fn knots(art: &Art, vertical: bool, extremes: bool) -> Vec<f64> {
    let coord = |p: &(f64, f64)| if vertical { p.0 } else { p.1 };
    let other = |p: &(f64, f64)| if vertical { p.1 } else { p.0 };
    let mut out = Vec::new();
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for sub in art.layers.iter().flat_map(|l| l.subs.iter()) {
        for i in 0..sub.len() {
            let a = &sub[i];
            let b = &sub[(i + 1) % sub.len()];
            lo = lo.min(coord(a));
            hi = hi.max(coord(a));
            if (coord(a) - coord(b)).abs() < 1e-9 && (other(a) - other(b)).abs() > 1e-9 {
                out.push(coord(a));
            }
        }
    }
    if extremes && lo.is_finite() {
        out.push(lo);
        out.push(hi);
    }
    out.sort_by(f64::total_cmp);
    out.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    out
}

/// Fits one axis: `hot` lands on `anchor`, every knot on a whole pixel (ties
/// away from the anchor), knots a unit or more apart at least a pixel apart.
fn fit_axis(knots: &[f64], hot: f64, anchor: f64, k: f64) -> Axis {
    let target = |u: f64| anchor + (u - hot) * k;
    let round_away = |p: f64| {
        if p >= anchor {
            (p + 0.5).floor()
        } else {
            (p - 0.5).ceil()
        }
    };
    // A real feature (a unit or more) keeps at least a pixel -- unless it
    // would be under half a pixel even unfitted (bitmaps under 12px), where
    // widening every one would overflow the bitmap.
    let keeps_apart = |du: f64| du >= 0.99 && du * k >= 0.5;
    // The hotspot only holds the next knot a pixel away if it is an edge
    // itself (the arrow's tip column is; the hand's fingertip centre is not,
    // and spacing from it would stretch the finger).
    let hot_is_edge = knots.iter().any(|&u| (u - hot).abs() < 1e-6);
    let fit = |dir: f64| {
        let mut out = Vec::new();
        let mut prev = (hot, anchor);
        let mut first = !hot_is_edge;
        let side = knots.iter().filter(|&&u| (u - hot) * dir > 1e-6);
        let ordered: Vec<f64> = if dir > 0.0 {
            side.copied().collect()
        } else {
            side.rev().copied().collect()
        };
        for u in ordered {
            let mut p = round_away(target(u));
            let floor = if std::mem::take(&mut first) || !keeps_apart((u - prev.0).abs()) {
                prev.1
            } else if dir > 0.0 {
                (prev.1 + 1.0).ceil()
            } else {
                (prev.1 - 1.0).floor()
            };
            if (p - floor) * dir < 0.0 {
                p = floor;
            }
            out.push((u, p));
            prev = (u, p);
        }
        out
    };
    let above = fit(1.0);
    let mut below = fit(-1.0);
    below.reverse();
    let mut all = below;
    all.push((hot, anchor));
    all.extend(above);
    Axis { knots: all, k }
}

/// The art's bounding box in grid units: `(min_x, min_y, max_x, max_y)`.
fn extent(art: &Art) -> [f64; 4] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for (x, y) in art.layers.iter().flat_map(|l| l.subs.iter()).flatten() {
        b[0] = b[0].min(*x);
        b[1] = b[1].min(*y);
        b[2] = b[2].max(*x);
        b[3] = b[3].max(*y);
    }
    b
}

/// Places `art` in a `size` bitmap; see the module docs.
pub(crate) fn place(art: &Art, size: i32) -> Placement {
    let s = f64::from(size);
    let rim = art
        .layers
        .iter()
        .map(|l| rim(l.stroke, size))
        .fold(1.0, f64::max);
    // What must stay inside the bitmap past the fill: the rim on every side,
    // the shadow's offset too on the right and bottom.
    let (lo_clear, hi_clear) = (rim, rim + shadow_offset(size));
    let downscaled = size < 24;
    let xs = knots(art, true, downscaled);
    let ys = knots(art, false, downscaled);
    let [lo_x, lo_y, hi_x, hi_y] = extent(art);
    let (hx, hy) = art.hot;
    // Mocu's hotspot scaled, on its sub-pixel phase: at and above Mocu's
    // scale a half-unit hotspot sits `k / 2` past a pixel corner (so every
    // whole-unit edge lands on a pixel edge where `k` is whole); below it, a
    // half-unit hotspot stays on a pixel centre, so symmetric art rounds to
    // odd, centred widths (the 16px resize shaft keeps 3px, not 2).
    let anchor = |h: f64, k: f64| {
        let frac = h - h.floor();
        let phase = if k < 1.0 {
            frac
        } else {
            (frac * k).rem_euclid(1.0)
        };
        (h * k - phase).round() + phase
    };
    let mut k = s / 24.0;
    let mut best = None;
    for _ in 0..24 {
        let mut x = fit_axis(&xs, hx, anchor(hx, k), k);
        let mut y = fit_axis(&ys, hy, anchor(hy, k), k);
        let mut fits = true;
        for (axis, lo, hi) in [(&mut x, lo_x, hi_x), (&mut y, lo_y, hi_y)] {
            // Shift by whole pixels (the fit is unchanged) to keep the
            // stroke and shadow inside; if both sides are short, shrink.
            let under = (lo_clear - axis.map(lo)).max(0.0).ceil();
            let over = (axis.map(hi) - (s - hi_clear)).max(0.0).ceil();
            axis.shift(under - over);
            fits &= axis.map(lo) >= lo_clear - 1e-9 && axis.map(hi) <= s - hi_clear + 1e-9;
        }
        best = Some(Placement {
            x,
            y,
            hot: (hx, hy),
        });
        if fits {
            break;
        }
        k *= 0.94;
    }
    best.expect("at least one round")
}
