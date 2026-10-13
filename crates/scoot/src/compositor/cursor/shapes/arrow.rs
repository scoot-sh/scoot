//! The arrow family: the pointer itself plus its three badges.
//!
//! Authored on the 24-unit grid ([`super::design`]): the silhouette below
//! keeps its round-3 fractions of the drawable box (notch, tail, barb --
//! those relations are proportional, so they scale with any size), while
//! absolute widths (the tail hint, the badge radii, the "?" stem) are grid
//! units. Every vertex sits on a pixel corner at 16px and on a grid corner
//! at 24px.

use super::super::raster::Shadow;
use super::paw;
use super::{Ctx, Prim, arrow_pad_br, arrow_shadow, arrow_tip, chamfer};

/// The arrow body polygon in bitmap coordinates: the original six-vertex
/// silhouette, tip first, clockwise. See the round-3 report for why these
/// fractions (notch 3px deep, tail edges vertical pixel columns, tail end
/// one flat row at 16px).
pub(crate) fn polygon(ctx: &Ctx) -> Vec<(f64, f64)> {
    let (tx, ty) = arrow_tip(ctx.size);
    let br = arrow_pad_br(ctx.size);
    let bw = f64::from(ctx.size - tx - br);
    let bh = f64::from(ctx.size - ty - br);
    let ox = f64::from(tx);
    let oy = f64::from(ty);
    // The tail's two long edges (notch->tail-left and barb->tail-right) are
    // parallel by construction: barb = tail-right - (tail-left - notch).
    let notch = (0.30 * bw, 0.50 * bh);
    let tail_left = (0.30 * bw, 0.90 * bh);
    let tail_right = (0.60 * bw, 1.00 * bh);
    let barb = (
        tail_right.0 - (tail_left.0 - notch.0),
        tail_right.1 - (tail_left.1 - notch.1),
    );
    vec![
        (0.0, 0.0),
        (0.0, 0.70 * bh),
        notch,
        tail_left,
        tail_right,
        barb,
    ]
    .into_iter()
    .map(|(x, y)| (ox + x, oy + y))
    .collect::<Vec<_>>()
}

/// The arrow's own chamfer: much smaller than the line-art rounding, so at
/// 16px the tip keeps a covered hotspot pixel and the tail corners stay
/// square; it grows with the bitmap so large cursors are lightly rounded.
fn arrow_round(size: i32) -> f64 {
    (f64::from(size) / 80.0).clamp(0.2, 2.0)
}

fn body_poly(ctx: &Ctx) -> Prim {
    Prim::Poly(chamfer(&polygon(ctx), arrow_round(ctx.size), Some(0)))
}

/// The pointer itself: the arrow plus an orange hint across the tail end.
pub(crate) fn arrow(ctx: &Ctx, alpha: f64) -> (Vec<Prim>, Vec<Prim>, Option<Shadow>) {
    let (tx, ty) = arrow_tip(ctx.size);
    let br = arrow_pad_br(ctx.size);
    let bw = f64::from(ctx.size - tx - br);
    let bh = f64::from(ctx.size - ty - br);
    let ox = f64::from(tx);
    let oy = f64::from(ty);
    // A short stripe just inside the tail's flat end: at 16px it covers the
    // end row's middle, a hint rather than a cap. 12/7 grid units wide.
    let w = (24.0 / 14.0 * ctx.u).max(super::design::MIN_STROKE);
    let hint = Prim::Capsule {
        ax: ox + 0.32 * bw,
        ay: oy + 0.93 * bh,
        bx: ox + 0.58 * bw,
        by: oy + 0.985 * bh,
        w,
    };
    (
        vec![body_poly(ctx)],
        vec![hint],
        arrow_shadow(ctx.size, alpha),
    )
}

/// Where a badge sits: just past the tail, inside the bottom-right inset.
/// Returns the badge centre in bitmap coordinates.
///
/// Each badge has its own anchor: the paw and the hourglass tuck beside the
/// tail's lower half, while the taller "?" rides slightly higher so its dot
/// stays inside the bitmap at every size (its extent below the centre is
/// the largest of the three -- see `help`).
fn badge_centre(ctx: &Ctx) -> (f64, f64) {
    let s = f64::from(ctx.size);
    (s * 0.68, s * 0.70)
}

/// Arrow plus a small paw badge at the lower right.
pub(crate) fn pointer_badge(ctx: &Ctx, alpha: f64) -> (Vec<Prim>, Vec<Prim>, Option<Shadow>) {
    let s = f64::from(ctx.size);
    // Badge radius is 3 grid units. Below 16px tuck the badge one pixel
    // further inside and shrink it: at 8px its halo touched the bitmap
    // edge, at 12px with blends.
    let grid_r = 3.0 * s / super::design::GRID;
    let (bx, by, r) = if ctx.size < 12 {
        (s * 0.64, s * 0.64, grid_r.clamp(1.2, 7.0))
    } else if ctx.size < 16 {
        (s * 0.66, s * 0.67, grid_r.clamp(1.4, 7.0))
    } else {
        let (bx, by) = badge_centre(ctx);
        (bx, by, grid_r.clamp(1.5, 7.0))
    };
    (
        vec![body_poly(ctx)],
        paw::prims(bx, by, r),
        arrow_shadow(ctx.size, alpha),
    )
}

/// Arrow plus a small hourglass badge: the frame in the body color, the sand
/// pile as the accent mark.
pub(crate) fn wait_badge(ctx: &Ctx, alpha: f64) -> (Vec<Prim>, Vec<Prim>, Option<Shadow>) {
    let u = ctx.u;
    // Tucked past the tail but clear of the bitmap edge: the badge must end
    // well before the bottom-right inset (outline plus shadow) or its lower
    // bar clips. Below 16px pull one pixel further inside (F3: blends
    // touched at 12px).
    let (bx, by) = if ctx.size < 12 {
        (f64::from(ctx.size) * 0.62, f64::from(ctx.size) * 0.60)
    } else if ctx.size < 16 {
        (f64::from(ctx.size) * 0.63, f64::from(ctx.size) * 0.59)
    } else {
        (f64::from(ctx.size) * 0.655, f64::from(ctx.size) * 0.63)
    };
    // A ~5.7x7.5 grid-unit bowtie at 24px: top and bottom bulbs in the body
    // color with capping bars, the sand as accent. Below 16px shrink the
    // whole badge slightly so its lower bar keeps its rim inset (F3).
    let badge_scale = if ctx.size < 16 { 0.8 } else { 1.0 };
    let w = 2.85 * u * badge_scale;
    let h = 3.75 * u * badge_scale;
    let waist = 0.45;
    let hourglass = vec![
        Prim::Poly(chamfer(
            &[
                (bx - w, by - h),
                (bx + w, by - h),
                (bx + waist * w, by),
                (bx + w, by + h),
                (bx - w, by + h),
                (bx - waist * w, by),
            ],
            ctx.round * 0.4,
            None,
        )),
        Prim::Capsule {
            ax: bx - w - 0.45 * u,
            ay: by - h,
            bx: bx + w + 0.45 * u,
            by: by - h,
            w: (1.35 * u).max(0.8),
        },
        Prim::Capsule {
            ax: bx - w - 0.45 * u,
            ay: by + h,
            bx: bx + w + 0.45 * u,
            by: by + h,
            w: (1.35 * u).max(0.8),
        },
    ];
    let mut mark = vec![Prim::Disk {
        cx: bx,
        cy: by + h * 0.55,
        r: (1.35 * u).max(0.7),
    }];
    // A falling grain between the waist and the pile.
    mark.push(Prim::Capsule {
        ax: bx,
        ay: by - h * 0.1,
        bx,
        by: by + h * 0.3,
        w: (0.75 * u).max(0.4),
    });
    let mut body = vec![body_poly(ctx)];
    body.extend(hourglass);
    (body, mark, arrow_shadow(ctx.size, alpha))
}

/// Arrow plus a "?" badge: the whole mark is accent, drawn as a capsule
/// chain (the curve), a stem and a dot. Compact by design: the dot sits only
/// ~3 units below the badge centre so the mark fits at 24px and past.
pub(crate) fn help(ctx: &Ctx, alpha: f64) -> (Vec<Prim>, Vec<Prim>, Option<Shadow>) {
    let u = ctx.u;
    // At 12px and below the badge halo touched the edge: pull the centre in
    // one pixel worth and let the mark shrink with it.
    let (bx, by) = if ctx.size < 12 {
        (f64::from(ctx.size) * 0.64, f64::from(ctx.size) * 0.60)
    } else if ctx.size < 16 {
        (f64::from(ctx.size) * 0.66, f64::from(ctx.size) * 0.63)
    } else {
        (f64::from(ctx.size) * 0.675, f64::from(ctx.size) * 0.66)
    };
    // "?" ~5.25 grid units wide, ~7.5 tall at 24px, centred on the badge
    // point. Bolder than round 4 (F4): a wider stem and a bigger dot so the
    // curve reads at 16px instead of dissolving into blends.
    let pts = [
        (-2.1, 0.225),
        (-1.5, -1.65),
        (0.3, -2.475),
        (2.025, -1.65),
        (2.175, 0.0),
        (0.825, 1.125),
        (0.15, 1.875),
    ];
    let w = (2.4 * u).max(super::design::MIN_STROKE);
    let mut mark = Vec::with_capacity(pts.len() + 1);
    for pair in pts.windows(2) {
        mark.push(Prim::Capsule {
            ax: bx + pair[0].0 * u,
            ay: by + pair[0].1 * u,
            bx: bx + pair[1].0 * u,
            by: by + pair[1].1 * u,
            w,
        });
    }
    mark.push(Prim::Capsule {
        ax: bx + 0.15 * u,
        ay: by + 1.8 * u,
        bx: bx + 0.15 * u,
        by: by + 3.0 * u,
        w,
    });
    mark.push(Prim::Disk {
        cx: bx + 0.15 * u,
        cy: by + 4.5 * u,
        r: (1.5 * u).max(0.7),
    });
    (vec![body_poly(ctx)], mark, arrow_shadow(ctx.size, alpha))
}
