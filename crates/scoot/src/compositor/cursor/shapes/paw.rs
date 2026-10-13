//! The paw: one geometry in two colorings.
//!
//! [`Shape::Busy`] wears it as the whole body (a paw-print cursor); the
//! pointer badge ([`arrow::pointer_badge`]) wears a small one as the accent
//! mark. Both go through [`prims`]; the caller decides which layer it lands
//! on.
//!
//! Authored on the 24-unit grid ([`super::design`]): the pad and toe radii
//! below are fractions of the print radius `r`, which is itself `5.76u`
//! for the busy print. The toe arc is deliberately broad (dx +/-0.78) to
//! match the concept sheet's spread without tracing it.

use super::design::TOE_DROP_BELOW;
use super::{Ctx, Prim};

/// A paw print centred on `(cx, cy)` with overall radius `r`: one main pad
/// plus three toes on an arc above it. All disks, so the mark stays round at
/// every size; the toes overlap the pad slightly so the union never pinches
/// into separate blobs. Toes narrower than [`TOE_DROP_BELOW`] are dropped
/// (the 8px pointer badge keeps its pad alone) instead of blurring away.
pub(crate) fn prims(cx: f64, cy: f64, r: f64) -> Vec<Prim> {
    let pad = Prim::Disk {
        cx,
        cy: cy + r * 0.30,
        r: r * 0.72,
    };
    let toe_r = r * 0.40;
    if toe_r < TOE_DROP_BELOW {
        return vec![pad];
    }
    let toe = |dx: f64, dy: f64| Prim::Disk {
        cx: cx + dx * r,
        cy: cy + dy * r,
        r: toe_r,
    };
    vec![pad, toe(-0.78, -0.42), toe(0.0, -0.70), toe(0.78, -0.42)]
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
    // Print radius 5.76 grid units; centred a fraction below the middle so
    // the toes keep their rim inset at small sizes.
    let r = 5.76 * ctx.u;
    let print = prims(ctx.c, ctx.c + 0.48 * ctx.u, r);
    (print.clone(), print, None)
}
