//! Shared geometry for the cursor family: the [`Prim`] vector primitives
//! and the rim width. [`generate`](super::generate) renders them.

use super::super::raster;

/// How far past the fill edge the rim reaches, in whole pixels: 1px up to
/// 31px (Mocu's own one-unit rim at 24), 2px from 32 (the default size on a
/// scale-2 output, so a hidpi cursor is the 1x drawing at twice the
/// resolution), capped at 3px so it never becomes a border. Whole pixels so
/// a grid-fitted edge's rim is as crisp as the edge; Mocu's literal rim at
/// 16px would be a two-thirds-pixel smear.
pub(crate) fn outline_width(size: i32) -> f64 {
    f64::from((size / 16).clamp(1, 3))
}

/// One vector primitive making up a shape's fill.
#[derive(Clone)]
pub(crate) enum Prim {
    /// A filled disk.
    Disk { cx: f64, cy: f64, r: f64 },
    /// A compound outline (even-odd across its subpaths, so holes work) --
    /// the flattened, placed Mocu art.
    Path(Vec<Vec<(f64, f64)>>),
}

impl Prim {
    pub(crate) fn distance(&self, px: f64, py: f64) -> f64 {
        match self {
            Prim::Disk { cx, cy, r } => raster::sd_circle(px, py, *cx, *cy, *r),
            Prim::Path(subs) => raster::sd_path(px, py, subs),
        }
    }

    /// The tight bounding box of the fill: `(min_x, min_y, max_x, max_y)`.
    pub(crate) fn bounds(&self) -> [f64; 4] {
        match self {
            Prim::Disk { cx, cy, r } => [cx - r, cy - r, cx + r, cy + r],
            Prim::Path(subs) => {
                let mut b = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for (x, y) in subs.iter().flatten() {
                    b[0] = b[0].min(*x);
                    b[1] = b[1].min(*y);
                    b[2] = b[2].max(*x);
                    b[3] = b[3].max(*y);
                }
                b
            }
        }
    }
}
