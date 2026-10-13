//! The paw: scoot's own mark, in two sizes.
//!
//! [`Shape::Busy`](super::Shape::Busy) is one whole paw print; the pointer
//! badge (see [`super::family`]) is a small one beside the arrow. Both go
//! through [`prims`], drawn as the body with the ginger accent over the
//! same silhouette.
//!
//! This is scoot's own geometry -- Mocu has no paw -- authored on the same
//! 24-unit grid as the Mocu art and drawn through the same rim, so it sits
//! in the family. The pad and toe radii below are fractions of the print
//! radius `r`, which is itself `5.76` grid units for the busy print. The toe
//! arc is deliberately broad (dx +/-0.78) to match the concept sheet's
//! spread without tracing it.

use super::family::Layer;
use super::{Ctx, Prim};

/// A paw toe narrower than this radius in pixels is dropped, leaving the
/// pad alone (the 8px pointer badge): a sub-pixel toe would only blur.
pub(crate) const TOE_DROP_BELOW: f64 = 0.6;

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
pub(crate) fn busy(ctx: &Ctx) -> Layer {
    // Print radius 5.76 grid units; centred a fraction below the middle so
    // the toes keep their rim inset at small sizes.
    let r = 5.76 * ctx.u;
    let print = prims(ctx.c, ctx.c + 0.48 * ctx.u, r);
    Layer {
        body: print.clone(),
        accent: print,
        accent_rim: true,
    }
}
