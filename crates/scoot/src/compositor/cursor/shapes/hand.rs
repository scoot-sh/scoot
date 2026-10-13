//! The hands: a pointing glove, an open grab hand and a grabbing fist.
//!
//! Pixel-art-leaning construction: every edge sits on a pixel boundary at
//! 16px (all coordinates below are multiples of 0.5px that resolve to whole
//! pixels at the default size), with only a light chamfer so corners do not
//! go razor-sharp at 256px. All three follow the body color like every other
//! shape (the concept sheet shows a white glove; a fixed-white hand would
//! ignore `cursor_color` and lose the rim contract on light content -- the
//! report states the deviation); the cuff and the knuckles carry the accent.

use super::{Ctx, Prim, chamfer};
use smithay::utils::{Logical, Point};

/// The pointing hand's fingertip in bitmap pixels: the left column of the
/// index finger's top row. Also [`Shape::hotspot`](super::Shape::hotspot)
/// for [`Shape::Hand`], so the two cannot drift apart.
pub(crate) fn fingertip(size: i32) -> Point<i32, Logical> {
    let s = f64::from(size) / 16.0;
    // Finger left edge at 6px, tip row at 2px in a 16px bitmap (3px below
    // 16px, where the fingertip drops one pixel for its rim inset -- see
    // `hand`).
    let tip_y = if size < 16 { 3.0 } else { 2.0 };
    ((6.0 * s).round() as i32, (tip_y * s).round() as i32).into()
}

/// One pixel-boundary rect as a chamfered polygon. The chamfer is capped:
/// the fingertip rect's corner is the hotspot pixel, and a large chamfer
/// would cut it away entirely at 256px (a 6px cut removes the whole pixel).
fn rect(x0: f64, y0: f64, x1: f64, y1: f64, r: f64) -> Prim {
    Prim::Poly(chamfer(
        &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)],
        r.min(1.5),
        None,
    ))
}

/// The pointing glove: index finger up, three folded fingers as one knuckle
/// block, thumb nub on the left, palm, wrist, and an orange cuff bar.
/// A 1px groove separates the index finger from the knuckle block (F4):
/// without it the two merge into one mitten at 16px.
pub(crate) fn hand(ctx: &Ctx) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    let s = f64::from(ctx.size) / 16.0;
    let r = (0.4 * s).max(0.2);
    let px = |x: f64| x * s;
    let py = |y: f64| y * s;
    // At 12px and below the blends touched the bitmap edge (F3): pull the
    // outer silhouette half a pixel inside by shrinking the palm/knuckle
    // extents there.
    let shrink = if ctx.size < 12 {
        0.5
    } else if ctx.size < 16 {
        0.25
    } else {
        0.0
    };
    // F3: at 8/12px the index fingertip plus its rim landed on row 0;
    // start one pixel lower below 16px so the rim keeps its inset.
    let finger_top = if ctx.size < 16 { 3.0 } else { 2.0 };
    let body = vec![
        // Index finger: columns 6-7, rows 2-7 at 16px.
        rect(px(6.0), py(finger_top), px(8.0), py(8.0), r),
        // Folded fingers: one knuckle block, rows 6-8. Starts at x=9 for
        // the 1px groove beside the index finger.
        rect(px(9.0), py(6.0), px(12.0 - shrink), py(9.0), r),
        // Palm.
        rect(px(5.0), py(8.0), px(12.0 - shrink), py(12.5), r),
        // Thumb nub on the left.
        rect(px(3.5), py(7.5), px(5.5), py(10.5), r),
    ];
    // Cuff: a 2px orange band across the wrist end (a 1px bar leaves no flat
    // core inside its own hairlines, so it would render all blend and read
    // grey rather than orange).
    let cuff = vec![rect(px(6.0), py(12.0), px(11.0), py(14.0), 0.0)];
    (body, cuff, None)
}

/// The open grab hand: four finger bars over a palm, thumb on the left. No
/// accent of its own (the sheet shows it plain); the closed fist carries the
/// orange knuckles instead, which is what tells the two apart at a glance.
/// A 1px groove separates the bars (F4): narrower bars on a wider spacing
/// than round 4's 0.5px valleys, so they read as fingers, not a mitten.
pub(crate) fn grab(ctx: &Ctx) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    let s = f64::from(ctx.size) / 16.0;
    let r = (0.4 * s).max(0.2);
    let px = |x: f64| x * s;
    let py = |y: f64| y * s;
    let shrink = if ctx.size < 16 { 0.25 } else { 0.0 };
    // F3: at 12px the thumb nub plus its rim touched column 0; tuck it
    // half a pixel inside below 16px.
    let thumb_x0 = if ctx.size < 16 { 3.0 } else { 2.5 };
    let mut body = vec![
        // Palm: columns 4-12, rows 8-13 at 16px.
        rect(px(4.0), py(8.0), px(12.0 - shrink), py(13.0), r),
        // Thumb nub on the left.
        rect(px(thumb_x0), py(8.0), px(4.5), py(11.0), r),
    ];
    // Four finger bars, each 1.25px wide on a 2.25px pitch at 16px: ~1px
    // valleys survive the rim as notches, which is what reads as
    // fingers rather than a mitten.
    for (i, fx) in [4.0, 6.25, 8.5, 10.75].into_iter().enumerate() {
        let top = if i == 1 { 4.0 } else { 4.5 };
        body.push(rect(px(fx), py(top), px(fx + 1.25), py(8.5), r));
    }
    (body, Vec::new(), None)
}

/// The grabbing fist: a rounded block with a thumb wrap and orange knuckle
/// ticks across the top. Biased up one row: the thumb mass sits low, so a
/// centred block centroids a pixel and a half too low (caught by the
/// centroid test).
pub(crate) fn grabbing(ctx: &Ctx) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    let s = f64::from(ctx.size) / 16.0;
    let r = (0.9 * s).max(0.3);
    let px = |x: f64| x * s;
    let py = |y: f64| y * s;
    // F3: at 12px the rim blends touched the edge; pull the outer extents
    // slightly inside there.
    let shrink = if ctx.size < 12 {
        0.5
    } else if ctx.size < 16 {
        0.25
    } else {
        0.0
    };
    // F3: at 8/12px the thumb capsule plus its rim touched column 0;
    // tuck it inside below 16px.
    let thumb_dx = if ctx.size < 16 { 0.75 } else { 0.0 };
    let body = vec![
        // Fist block: columns 4-12, rows 4-11 at 16px.
        rect(px(4.0), py(4.0), px(12.0 - shrink), py(11.0), r),
        // Thumb wrap: diagonal bar across the lower left.
        Prim::Capsule {
            ax: px(3.5 + thumb_dx),
            ay: py(8.5),
            bx: px(7.5),
            by: py(10.5),
            w: (1.8 * s).max(1.5),
        },
    ];
    // Knuckles: three short orange ticks across the top of the fist, sunk
    // into its face (flush with the top edge they drown in the rim blends).
    let mut knuckles = Vec::with_capacity(3);
    for kx in [5.25, 7.5, 9.75] {
        knuckles.push(Prim::Capsule {
            ax: px(kx),
            ay: py(5.5),
            bx: px(kx),
            by: py(7.5),
            w: (1.25 * s).max(1.0),
        });
    }
    (body, knuckles, None)
}
