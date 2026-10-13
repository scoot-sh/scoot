//! The arrow family: the pointer itself plus its three badges.
//!
//! The arrow is the round-3 six-vertex silhouette (straight vertical left
//! edge, exact 45-degree hypotenuse, concave notch, short flat-ended tail
//! with parallel vertical edges), every vertex on a pixel corner at 16px.
//! The badges reuse it unchanged and add one mark at the lower right: the
//! paw ([`Shape::PointerBadge`]), the hourglass ([`Shape::WaitBadge`]) or
//! the "?" ([`Shape::Help`]).

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
    // end row's middle, a hint rather than a cap.
    let w = (f64::from(ctx.size) / 14.0).max(1.0);
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
    let (bx, by) = badge_centre(ctx);
    let r = (f64::from(ctx.size) / 8.0).clamp(1.5, 7.0);
    (
        vec![body_poly(ctx)],
        paw::prims(bx, by, r),
        arrow_shadow(ctx.size, alpha),
    )
}

/// Arrow plus a small hourglass badge: the frame in the body color, the sand
/// pile as the accent mark.
pub(crate) fn wait_badge(ctx: &Ctx, alpha: f64) -> (Vec<Prim>, Vec<Prim>, Option<Shadow>) {
    let s = f64::from(ctx.size) / 16.0;
    // Tucked past the tail but clear of the bitmap edge: the badge must end
    // well before the bottom-right inset (outline plus shadow) or its lower
    // bar clips.
    let bx = f64::from(ctx.size) * 0.655;
    let by = f64::from(ctx.size) * 0.63;
    // A ~3.8x5px bowtie at 16px: top and bottom bulbs in the body color
    // with capping bars, the sand as accent.
    let w = 1.9 * s;
    let h = 2.5 * s;
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
            ax: bx - w - 0.3 * s,
            ay: by - h,
            bx: bx + w + 0.3 * s,
            by: by - h,
            w: (0.9 * s).max(0.8),
        },
        Prim::Capsule {
            ax: bx - w - 0.3 * s,
            ay: by + h,
            bx: bx + w + 0.3 * s,
            by: by + h,
            w: (0.9 * s).max(0.8),
        },
    ];
    let mut mark = vec![Prim::Disk {
        cx: bx,
        cy: by + h * 0.55,
        r: (0.9 * s).max(0.7),
    }];
    // A falling grain between the waist and the pile.
    mark.push(Prim::Capsule {
        ax: bx,
        ay: by - h * 0.1,
        bx,
        by: by + h * 0.3,
        w: (0.5 * s).max(0.4),
    });
    let mut body = vec![body_poly(ctx)];
    body.extend(hourglass);
    (body, mark, arrow_shadow(ctx.size, alpha))
}

/// Arrow plus a "?" badge: the whole mark is accent, drawn as a capsule
/// chain (the curve), a stem and a dot. Compact by design: the dot sits only
/// ~3 units below the badge centre so the mark fits at 24px and past.
pub(crate) fn help(ctx: &Ctx, alpha: f64) -> (Vec<Prim>, Vec<Prim>, Option<Shadow>) {
    let s = f64::from(ctx.size) / 16.0;
    let bx = f64::from(ctx.size) * 0.675;
    let by = f64::from(ctx.size) * 0.66;
    // "?" ~3.5px wide, ~5px tall at 16px, centred on the badge point.
    let pts = [
        (-1.3, 0.1),
        (-0.9, -1.0),
        (0.2, -1.5),
        (1.2, -1.0),
        (1.3, 0.0),
        (0.5, 0.7),
        (0.1, 1.2),
    ];
    let w = (1.3 * s).max(1.0);
    let mut mark = Vec::with_capacity(pts.len() + 1);
    for pair in pts.windows(2) {
        mark.push(Prim::Capsule {
            ax: bx + pair[0].0 * s,
            ay: by + pair[0].1 * s,
            bx: bx + pair[1].0 * s,
            by: by + pair[1].1 * s,
            w,
        });
    }
    mark.push(Prim::Capsule {
        ax: bx + 0.1 * s,
        ay: by + 1.2 * s,
        bx: bx + 0.1 * s,
        by: by + 2.0 * s,
        w,
    });
    mark.push(Prim::Disk {
        cx: bx + 0.1 * s,
        cy: by + 3.0 * s,
        r: (0.85 * s).max(0.65),
    });
    (vec![body_poly(ctx)], mark, arrow_shadow(ctx.size, alpha))
}
