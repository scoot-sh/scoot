//! The symbols: crosshairs, the "not allowed" ring and the magnifiers.
//!
//! The crosshair keeps its predecessors' plus in the body color with an
//! orange centre dot; precision is the same idea drawn thinner with no dot,
//! which is what tells the two apart. The ring is body-colored (it follows
//! `cursor_color` like every other silhouette) with an orange slash; the
//! magnifiers are a ring plus handle with an orange plus or minus in the lens.

use super::{Ctx, Prim, span};

/// A plus in the body color with an orange centre dot.
pub(crate) fn crosshair(ctx: &Ctx) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    let inset = (ctx.ow + ctx.w / 2.0 + 1.0).ceil();
    let (near, far) = span(ctx.size, inset);
    let body = vec![
        Prim::Capsule {
            ax: ctx.c,
            ay: near,
            bx: ctx.c,
            by: far,
            w: ctx.w,
        },
        Prim::Capsule {
            ax: near,
            ay: ctx.c,
            bx: far,
            by: ctx.c,
            w: ctx.w,
        },
    ];
    let mark = vec![Prim::Disk {
        cx: ctx.c,
        cy: ctx.c,
        r: (ctx.w * 0.36).max(0.9),
    }];
    (body, mark, None)
}

/// A thinner plain crosshair, for precise picking.
pub(crate) fn precision(ctx: &Ctx) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    let w = (f64::from(ctx.size) / 12.0).max(1.2);
    let inset = (ctx.ow + w / 2.0 + 1.0).ceil();
    let (near, far) = span(ctx.size, inset);
    let body = vec![
        Prim::Capsule {
            ax: ctx.c,
            ay: near,
            bx: ctx.c,
            by: far,
            w,
        },
        Prim::Capsule {
            ax: near,
            ay: ctx.c,
            bx: far,
            by: ctx.c,
            w,
        },
    ];
    (body, Vec::new(), None)
}

/// A ring with an orange slash through it.
pub(crate) fn not_allowed(
    ctx: &Ctx,
) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    let inset = (ctx.ow + ctx.w / 2.0 + 1.0).ceil();
    let (_near, far) = span(ctx.size, inset);
    let radius = far - ctx.c;
    if radius < 1.0 {
        return (
            vec![Prim::Disk {
                cx: ctx.c,
                cy: ctx.c,
                r: ctx.w / 2.0,
            }],
            Vec::new(),
            None,
        );
    }
    // A bolder ring and slash than the hairline strokes elsewhere: at 16px a
    // thin ring reads as wire, not as "no".
    let bold = ctx.w + 0.4;
    // Long enough to land on the ring's own stroke, so the two merge into
    // one mark rather than floating apart.
    let reach = radius * 0.85;
    let body = vec![Prim::Ring {
        cx: ctx.c,
        cy: ctx.c,
        r: radius,
        w: bold,
    }];
    let mark = vec![Prim::Capsule {
        ax: ctx.c - reach,
        ay: ctx.c + reach,
        bx: ctx.c + reach,
        by: ctx.c - reach,
        w: (bold * 0.8).max(1.2),
    }];
    (body, mark, None)
}

/// A magnifier with a plus or a minus in the lens.
fn zoom(ctx: &Ctx, plus: bool) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    let inset = (ctx.ow + ctx.w / 2.0 + 1.0).ceil();
    let (near, far) = span(ctx.size, inset);
    let span_len = far - near;
    // Lens ring centred on the bitmap middle (which is also the hotspot, so
    // the lens aims where the pointer points), handle running down-right to
    // the span's far corner.
    let r = span_len * 0.30;
    let (cx, cy) = (ctx.c, ctx.c);
    let w = (ctx.w * 0.9).max(1.6);
    let mut body = vec![Prim::Ring { cx, cy, r, w }];
    // Handle from the ring's edge to the span's far corner.
    let hx = cx + r * 0.72;
    let hy = cy + r * 0.72;
    body.push(Prim::Capsule {
        ax: hx,
        ay: hy,
        bx: far,
        by: far,
        w,
    });
    // Plus or minus in the lens, in the accent color.
    let arm = r * 0.55;
    let mw = (ctx.w * 0.6).max(1.2);
    let mut mark = vec![Prim::Capsule {
        ax: cx - arm,
        ay: cy,
        bx: cx + arm,
        by: cy,
        w: mw,
    }];
    if plus {
        mark.push(Prim::Capsule {
            ax: cx,
            ay: cy - arm,
            bx: cx,
            by: cy + arm,
            w: mw,
        });
    }
    (body, mark, None)
}

/// A magnifier with an orange plus.
pub(crate) fn zoom_in(ctx: &Ctx) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    zoom(ctx, true)
}

/// A magnifier with an orange minus.
pub(crate) fn zoom_out(ctx: &Ctx) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    zoom(ctx, false)
}
