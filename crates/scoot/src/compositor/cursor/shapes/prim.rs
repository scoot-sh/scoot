//! Shared geometry for the cursor family: the [`Prim`]
//! vector primitives, the [`chamfer`] corner rounding and the inset helpers.
//! Every family module builds its shapes out of these (plus the [`Ctx`]
//! metrics from the parent); [`generate`](super::generate)
//! renders them.

use super::super::raster::{self, Shadow};

/// How far past the fill edge the rim reaches, in pixels: 1px at the
/// default size, growing slightly with the bitmap so a large cursor keeps a
/// visible rim, capped so it never becomes a border. In grid units
/// ([`super::design`]) that is `1.5u` clamped to `1.0..=2.5`.
pub(crate) fn outline_width(size: i32) -> f64 {
    (f64::from(size) / 16.0).clamp(1.0, 2.5)
}

/// How wide every line-art stroke is drawn, in pixels: 2px at the default
/// size so a stroke carries a solid core inside its own rim on both
/// sides (a 1.6px stem's core dissolves into edge blends at 16px), growing
/// from there so a large cursor is a larger drawing rather than the same
/// hairline on a bigger grid. In grid units that is `8/3u` floored at 2px
/// (see [`super::design::MIN_STROKE`]).
pub(crate) fn stroke_width(size: i32) -> f64 {
    (f64::from(size) / 9.0).max(2.0)
}

/// The arrow tip's position in the bitmap: `ceil(outline + 1)` in both axes,
/// room for the rim band plus one pixel of anti-aliasing bleed. The
/// bitmap's top-left corner is transparent padding, never fill -- which is
/// why the hotspot is the tip, not `(0, 0)`.
pub(crate) fn arrow_tip(size: i32) -> (i32, i32) {
    let tip = (outline_width(size) + 1.0).ceil() as i32;
    (tip.max(1), tip.max(1))
}

/// The arrow's bottom-right inset: rim plus the shadow's offset and
/// feather plus bleed. Small bitmaps carry no shadow (it would not fit), so
/// they only reserve the rim there.
pub(crate) fn arrow_pad_br(size: i32) -> i32 {
    let ow = outline_width(size);
    if size < 12 {
        (ow + 1.0).ceil() as i32
    } else {
        let k = (f64::from(size) / 16.0).clamp(1.0, 2.0);
        (ow + 1.0 * k + 1.0 * k + 1.0).ceil() as i32
    }
}

/// The arrow's soft shadow, or `None` on bitmaps too small to hold one.
/// Peak alpha is deliberately low (28% of the fill's own alpha): present on
/// a light background, quiet on a dark one, never a second silhouette.
pub(crate) fn arrow_shadow(size: i32, alpha: f64) -> Option<Shadow> {
    if size < 12 || alpha <= 0.0 {
        return None;
    }
    let k = (f64::from(size) / 16.0).clamp(1.0, 2.0);
    Some(Shadow {
        dx: 0.75 * k,
        dy: 1.0 * k,
        feather: 1.0 * k,
        alpha: 0.28 * alpha,
    })
}

/// The near/far ends of every line-art stroke: mirrored about the hotspot
/// (`size / 2`) and `inset` pixels inside the bitmap edge (rim plus half
/// the stroke plus bleed). Both land on the half-pixel grid -- the middle of
/// a pixel column, not the crack between two -- so a stroke covers whole
/// pixels instead of straddling two columns greyly. When the bitmap is too
/// small for that, both collapse onto the hotspot and the shape degrades to
/// a dot -- the honest outcome at tiny sizes.
pub(crate) fn span(size: i32, inset: f64) -> (f64, f64) {
    let near = inset + 0.5;
    let far = f64::from(size) - inset - 0.5;
    if far <= near {
        let h = f64::from(size / 2) + 0.5;
        (h, h)
    } else {
        (near, far)
    }
}

/// One vector primitive making up a shape's fill.
#[derive(Clone)]
pub(crate) enum Prim {
    /// A round-capped stroke from `a` to `b`, `w` pixels wide.
    Capsule {
        ax: f64,
        ay: f64,
        bx: f64,
        by: f64,
        w: f64,
    },
    /// A filled polygon (the arrow body, an arrowhead).
    Poly(Vec<(f64, f64)>),
    /// A ring around `(cx, cy)` of radius `r`, `w` pixels wide.
    Ring { cx: f64, cy: f64, r: f64, w: f64 },
    /// A filled disk.
    Disk { cx: f64, cy: f64, r: f64 },
}

impl Prim {
    pub(crate) fn distance(&self, px: f64, py: f64) -> f64 {
        match self {
            Prim::Capsule { ax, ay, bx, by, w } => {
                raster::sd_segment(px, py, *ax, *ay, *bx, *by) - w / 2.0
            }
            Prim::Poly(verts) => raster::sd_poly(px, py, verts),
            Prim::Ring { cx, cy, r, w } => {
                (raster::sd_circle(px, py, *cx, *cy, *r)).abs() - w / 2.0
            }
            Prim::Disk { cx, cy, r } => raster::sd_circle(px, py, *cx, *cy, *r),
        }
    }

    /// The tight bounding box of the fill: `(min_x, min_y, max_x, max_y)`.
    pub(crate) fn bounds(&self) -> [f64; 4] {
        match self {
            Prim::Capsule { ax, ay, bx, by, w } => {
                let r = w / 2.0;
                [
                    ax.min(*bx) - r,
                    ay.min(*by) - r,
                    ax.max(*bx) + r,
                    ay.max(*by) + r,
                ]
            }
            Prim::Poly(verts) => {
                let mut b = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for (x, y) in verts {
                    b[0] = b[0].min(*x);
                    b[1] = b[1].min(*y);
                    b[2] = b[2].max(*x);
                    b[3] = b[3].max(*y);
                }
                b
            }
            Prim::Ring { cx, cy, r, w } => {
                let e = r + w / 2.0;
                [cx - e, cy - e, cx + e, cy + e]
            }
            Prim::Disk { cx, cy, r } => [cx - r, cy - r, cx + r, cy + r],
        }
    }
}

/// Rounds the convex corners of `verts` with a chamfer of `r` pixels: each
/// sharp convex vertex becomes a short flat edge, which reads as a rounded
/// corner once supersampled. Concave corners (the arrow's notch) stay sharp.
/// The `tip` vertex -- the arrow's own point, or an arrowhead's -- is only
/// ever chamfered by up to 1px: it is the click point, so it must stay where
/// the hotspot says it is no matter how large the bitmap gets.
/// `r` is clamped to 40% of either incident edge so tiny polygons cannot
/// invert.
pub(crate) fn chamfer(verts: &[(f64, f64)], r: f64, tip: Option<usize>) -> Vec<(f64, f64)> {
    if verts.len() < 3 || r <= 0.0 {
        return verts.to_vec();
    }
    let area: f64 = verts
        .iter()
        .enumerate()
        .map(|(i, (x, y))| {
            let (nx, ny) = verts[(i + 1) % verts.len()];
            x * ny - nx * y
        })
        .sum();
    let mut out = Vec::with_capacity(verts.len() + 4);
    for i in 0..verts.len() {
        let (px, py) = verts[(i + verts.len() - 1) % verts.len()];
        let (vx, vy) = verts[i];
        let (nx, ny) = verts[(i + 1) % verts.len()];
        let cross = (vx - px) * (ny - vy) - (vy - py) * (nx - vx);
        // Convex when the turn matches the winding.
        if cross * area <= 0.0 {
            out.push((vx, vy));
            continue;
        }
        let len_prev = ((vx - px) * (vx - px) + (vy - py) * (vy - py)).sqrt();
        let len_next = ((nx - vx) * (nx - vx) + (ny - vy) * (ny - vy)).sqrt();
        if len_prev <= 0.0 || len_next <= 0.0 {
            out.push((vx, vy));
            continue;
        }
        let mut corner = r;
        if tip == Some(i) {
            corner = corner.min(1.0);
        }
        let cut = corner.min(0.4 * len_prev).min(0.4 * len_next);
        out.push((
            vx + (px - vx) / len_prev * cut,
            vy + (py - vy) / len_prev * cut,
        ));
        out.push((
            vx + (nx - vx) / len_next * cut,
            vy + (ny - vy) / len_next * cut,
        ));
    }
    out
}
