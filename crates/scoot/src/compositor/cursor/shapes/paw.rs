//! The paw: one geometry in two colorings.
//!
//! [`Shape::Busy`] wears it as the whole body (a paw-print cursor); the
//! pointer badge ([`arrow::pointer_badge`]) wears a small one as the accent
//! mark. Both go through [`prims`]; the caller decides which layer it lands
//! on.

use super::{Ctx, Prim};

/// A paw print centred on `(cx, cy)` with overall radius `r`: one main pad
/// plus three toes on an arc above it. All disks, so the mark stays round at
/// every size; the toes overlap the pad slightly so the union never pinches
/// into separate blobs.
pub(crate) fn prims(cx: f64, cy: f64, r: f64) -> Vec<Prim> {
    let pad = Prim::Disk {
        cx,
        cy: cy + r * 0.30,
        r: r * 0.72,
    };
    let toe = |dx: f64, dy: f64| Prim::Disk {
        cx: cx + dx * r,
        cy: cy + dy * r,
        r: r * 0.40,
    };
    vec![pad, toe(-0.72, -0.42), toe(0.0, -0.68), toe(0.72, -0.42)]
}

/// The busy cursor: a single paw print, in the accent color over a
/// body-colored underlay of the same silhouette.
///
/// The concept sheet shows an all-orange print, so with the accent enabled
/// that is what draws: the accent fill covers the body's own print exactly
/// (same primitives), leaving only its rim-color hairline as a halo. With
/// the accent disabled (`None`) the body's print still draws on its own, so
/// the shape never vanishes -- it simply follows the body color like every
/// other shape.
pub(crate) fn busy(ctx: &Ctx) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    let s = f64::from(ctx.size);
    let r = s * 0.24;
    let print = prims(ctx.c, ctx.c + s * 0.02, r);
    (print.clone(), print, None)
}
