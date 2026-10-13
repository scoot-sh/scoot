//! The move/resize family: shafts with heads in the body color, the orange
//! centre mark as accent.
//!
//! Authored on the 24-unit grid ([`super::design`]): head reach, half-width
//! and diagonal-head values below are grid units (4.8/4.0/6.0u), the same
//! pixels as the old `size / 5`, `size / 6`, `size / 4`.
//!
//! Orthogonal resizes carry a solid centre line, diagonals a dashed one
//! (their 45-degree shafts have no solid core to read by, so the dashes must
//! flare as separate ticks to say which way they point); move carries a
//! centre dot; the splits are two triangles around an orange divider.

use super::{Ctx, Prim, chamfer, span};

/// The common line-art metrics: shaft ends on the half-pixel grid, head
/// reach and half-width clamped to the span so heads cannot outgrow their
/// own shaft at any size.
struct Metrics {
    near: f64,
    far: f64,
    head: f64,
    half_base: f64,
    diag_head: f64,
    c: f64,
}

fn metrics(ctx: &Ctx) -> Metrics {
    let inset = (ctx.ow + ctx.w / 2.0 + 1.0).ceil();
    let (near, far) = span(ctx.size, inset);
    Metrics {
        near,
        far,
        head: (4.8 * ctx.u).max(3.0).min((far - near) / 2.0),
        half_base: (4.0 * ctx.u).max(2.0).min(ctx.c - near).min(far - ctx.c),
        diag_head: (6.0 * ctx.u).max(4.0).min((far - near) / 2.0),
        c: ctx.c,
    }
}

/// A triangular head with its tip at `(tx, ty)` opening toward `(dx, dy)`.
fn head(ctx: &Ctx, m: &Metrics, tx: f64, ty: f64, dx: f64, dy: f64, len: f64) -> Prim {
    // Head: plain triangle, tip capped at 1px -- the click-point rule from
    // `chamfer`.
    let bx = tx + dx * len;
    let by = ty + dy * len;
    let hw = m.half_base;
    Prim::Poly(chamfer(
        &[
            (tx, ty),
            (bx - dy * hw, by + dx * hw),
            (bx + dy * hw, by - dx * hw),
        ],
        ctx.round * 0.5,
        Some(0),
    ))
}

/// A vertical double-headed arrow.
pub(crate) fn resize_ns(ctx: &Ctx) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    let m = metrics(ctx);
    let body = vec![
        Prim::Capsule {
            ax: m.c,
            ay: m.near,
            bx: m.c,
            by: m.far,
            w: ctx.w,
        },
        head(ctx, &m, m.c, m.near, 0.0, 1.0, m.head),
        head(ctx, &m, m.c, m.far, 0.0, -1.0, m.head),
    ];
    // Solid orange centre line over the shaft's middle 60%.
    let lo = m.near + (m.far - m.near) * 0.20;
    let hi = m.far - (m.far - m.near) * 0.20;
    let mark = vec![Prim::Capsule {
        ax: m.c,
        ay: lo,
        bx: m.c,
        by: hi,
        w: (ctx.w * 0.45).max(0.9),
    }];
    (body, mark, None)
}

/// A horizontal double-headed arrow.
pub(crate) fn resize_ew(ctx: &Ctx) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    let m = metrics(ctx);
    let body = vec![
        Prim::Capsule {
            ax: m.near,
            ay: m.c,
            bx: m.far,
            by: m.c,
            w: ctx.w,
        },
        head(ctx, &m, m.near, m.c, 1.0, 0.0, m.head),
        head(ctx, &m, m.far, m.c, -1.0, 0.0, m.head),
    ];
    let lo = m.near + (m.far - m.near) * 0.20;
    let hi = m.far - (m.far - m.near) * 0.20;
    let mark = vec![Prim::Capsule {
        ax: lo,
        ay: m.c,
        bx: hi,
        by: m.c,
        w: (ctx.w * 0.45).max(0.9),
    }];
    (body, mark, None)
}

/// One diagonal shaft with corner heads plus two dashed accent ticks.
fn diagonal(
    ctx: &Ctx,
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
    corner_a: [(f64, f64); 3],
    corner_b: [(f64, f64); 3],
) -> (Vec<Prim>, Vec<Prim>) {
    let body = vec![
        Prim::Capsule {
            ax,
            ay,
            bx,
            by,
            w: ctx.w,
        },
        Prim::Poly(chamfer(&corner_a, ctx.round * 0.5, Some(0))),
        Prim::Poly(chamfer(&corner_b, ctx.round * 0.5, Some(0))),
    ];
    // Two dashed ticks along the shaft's middle, clear of the heads (which
    // occupy roughly the outer quarters): at 16px those are ~1.5px dashes
    // with background between.
    let tick = |t0: f64, t1: f64| Prim::Capsule {
        ax: ax + (bx - ax) * t0,
        ay: ay + (by - ay) * t0,
        bx: ax + (bx - ax) * t1,
        by: ay + (by - ay) * t1,
        w: (ctx.w * 0.45).max(0.9),
    };
    (body, vec![tick(0.36, 0.47), tick(0.53, 0.64)])
}

/// A double-headed arrow from top-left to bottom-right.
pub(crate) fn resize_nwse(
    ctx: &Ctx,
) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    let m = metrics(ctx);
    let (body, mark) = diagonal(
        ctx,
        m.near,
        m.near,
        m.far,
        m.far,
        [
            (m.near, m.near),
            (m.near + m.diag_head, m.near),
            (m.near, m.near + m.diag_head),
        ],
        [
            (m.far, m.far),
            (m.far, m.far - m.diag_head),
            (m.far - m.diag_head, m.far),
        ],
    );
    (body, mark, None)
}

/// A double-headed arrow from bottom-left to top-right.
pub(crate) fn resize_nesw(
    ctx: &Ctx,
) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    let m = metrics(ctx);
    let (body, mark) = diagonal(
        ctx,
        m.far,
        m.near,
        m.near,
        m.far,
        [
            (m.far, m.near),
            (m.far, m.near + m.diag_head),
            (m.far - m.diag_head, m.near),
        ],
        [
            (m.near, m.far),
            (m.near, m.far - m.diag_head),
            (m.near + m.diag_head, m.far),
        ],
    );
    (body, mark, None)
}

/// A connected cross with four plain triangular heads and an orange centre
/// dot -- the same structure as a four-arrow sign: the shafts meet at the
/// middle, the heads flare past them, and the diagonal notches between
/// neighbouring head bases stay open background so four arrows read.
pub(crate) fn move_shape(
    ctx: &Ctx,
) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    // The span uses a tighter inset than the strokes (tips need only rim
    // plus bleed) so the heads fit in 16px at all.
    let inset = ctx.ow + 0.5;
    let (near, far) = span(ctx.size, inset);
    let span_len = far - near;
    // 24/5.5 and 3 grid units: the same pixels as the old size/5.5, size/8.
    let mhead = (24.0 / 5.5 * ctx.u).max(3.0).min(span_len / 2.0);
    let mhalf = (3.0 * ctx.u).max(1.8).min(ctx.c - near).min(far - ctx.c);
    // The shafts stop half a stroke short of the head tips: round caps
    // overhang their endpoints by `w / 2`, which at large sizes dwarfs the
    // tip inset and would spill fill off the bitmap edge. Ending flush with
    // the tip plane keeps every cap inside (the heads cover the difference).
    let (snear, sfar) = (
        (near + ctx.w / 2.0).min(ctx.c),
        (far - ctx.w / 2.0).max(ctx.c),
    );
    let c = ctx.c;
    let mut body = vec![
        Prim::Capsule {
            ax: c,
            ay: snear,
            bx: c,
            by: sfar,
            w: ctx.w,
        },
        Prim::Capsule {
            ax: snear,
            ay: c,
            bx: sfar,
            by: c,
            w: ctx.w,
        },
    ];
    // Reuse the shared head builder through a throwaway metrics: the head
    // length/half-width here are move-specific (narrower, so the diagonal
    // notches stay open).
    let m = Metrics {
        near,
        far,
        head: mhead,
        half_base: mhalf,
        diag_head: mhead,
        c,
    };
    for (tx, ty, dx, dy) in [
        (c, near, 0.0, 1.0),
        (c, far, 0.0, -1.0),
        (near, c, 1.0, 0.0),
        (far, c, -1.0, 0.0),
    ] {
        body.push(head(ctx, &m, tx, ty, dx, dy, mhead));
    }
    let mark = vec![Prim::Disk {
        cx: c,
        cy: c,
        r: (ctx.w * 0.38).max(1.0),
    }];
    (body, mark, None)
}

/// Two triangles around a vertical orange divider. Slimmer than round 4
/// (F4): narrower triangles and a thinner divider so the pair reads as a
/// splitter, not an eye.
pub(crate) fn split_v(ctx: &Ctx) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    let m = metrics(ctx);
    let h = (4.0 * ctx.u).max(2.8).min((m.far - m.near) / 2.0 - 0.5);
    let gap = (ctx.w * 0.35).max(0.8) / 2.0 + ctx.aow + 0.9;
    let left = Prim::Poly(chamfer(
        &[(m.near, m.c), (m.c - gap, m.c - h), (m.c - gap, m.c + h)],
        ctx.round * 0.5,
        Some(0),
    ));
    let right = Prim::Poly(chamfer(
        &[(m.far, m.c), (m.c + gap, m.c + h), (m.c + gap, m.c - h)],
        ctx.round * 0.5,
        Some(0),
    ));
    let mark = vec![Prim::Capsule {
        ax: m.c,
        ay: m.c - h,
        bx: m.c,
        by: m.c + h,
        w: (ctx.w * 0.35).max(0.8),
    }];
    (vec![left, right], mark, None)
}

/// Two triangles around a horizontal orange divider. Slimmer than round 4
/// (F4): see `split_v`.
pub(crate) fn split_h(ctx: &Ctx) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    let m = metrics(ctx);
    let h = (4.0 * ctx.u).max(2.8).min((m.far - m.near) / 2.0 - 0.5);
    let gap = (ctx.w * 0.35).max(0.8) / 2.0 + ctx.aow + 0.9;
    let top = Prim::Poly(chamfer(
        &[(m.c, m.near), (m.c - h, m.c - gap), (m.c + h, m.c - gap)],
        ctx.round * 0.5,
        Some(0),
    ));
    let bottom = Prim::Poly(chamfer(
        &[(m.c, m.far), (m.c + h, m.c + gap), (m.c - h, m.c + gap)],
        ctx.round * 0.5,
        Some(0),
    ));
    let mark = vec![Prim::Capsule {
        ax: m.c - h,
        ay: m.c,
        bx: m.c + h,
        by: m.c,
        w: (ctx.w * 0.35).max(0.8),
    }];
    (vec![top, bottom], mark, None)
}
