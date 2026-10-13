//! The hands: a pointing glove, an open grab hand and a grabbing fist.
//!
//! Authored on the 24-unit grid ([`super::design`]): every coordinate below
//! is in 24ths of the bitmap edge via [`Ctx::g`], so grid 9 is pixel column
//! 6 at 16px and column 13.5 at 24px -- one drawing, every size. (The
//! pre-grid code wrote the same pixels as multiples of `size / 16`; the
//! values here are those multiplied by 1.5, verified byte-identical at
//! 16/24/32 except for the intended F4 groove and F3 tiny-size tucks.)
//!
//! Pixel-art-leaning construction: every edge sits on a pixel boundary at
//! 16px (all grid coordinates below resolve to whole or half pixels at the
//! default size), with only a light chamfer so corners do not go
//! razor-sharp at 256px. All three follow the body color like every other
//! shape (the concept sheet shows a white glove; a fixed-white hand would
//! ignore `cursor_color` and lose the rim contract on light content -- the
//! report states the deviation); the cuff and the knuckles carry the accent.

use super::{Ctx, Prim, chamfer};
use smithay::utils::{Logical, Point};

/// The pointing hand's fingertip in bitmap pixels: the left column of the
/// index finger's top row. Also [`Shape::hotspot`](super::Shape::hotspot)
/// for [`Shape::Hand`], so the two cannot drift apart.
pub(crate) fn fingertip(size: i32) -> Point<i32, Logical> {
    // Finger left edge at grid 9, tip row at grid 3 (4.5 below 16px, where
    // the fingertip drops one grid unit for its rim inset -- see `hand`).
    let tip_g = if size < 16 { 4.5 } else { 3.0 };
    (
        (9.0 * f64::from(size) / super::design::GRID).round() as i32,
        (tip_g * f64::from(size) / super::design::GRID).round() as i32,
    )
        .into()
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
    let r = (0.6 * ctx.u).max(0.2);
    // At 12px and below the blends touched the bitmap edge (F3): pull the
    // outer silhouette inside by shrinking the palm/knuckle extents there
    // (grid units).
    let shrink = if ctx.size < 12 {
        0.75
    } else if ctx.size < 16 {
        0.375
    } else {
        0.0
    };
    // F3: at 8/12px the index fingertip plus its rim landed on row 0;
    // start one grid unit lower below 16px so the rim keeps its inset.
    let finger_top = if ctx.size < 16 { 4.5 } else { 3.0 };
    let body = vec![
        // Index finger: grid 9-12, rows 3-12 at 24px (= columns 6-8 at 16px).
        rect(ctx.g(9.0), ctx.g(finger_top), ctx.g(12.0), ctx.g(12.0), r),
        // Folded fingers: one knuckle block. Starts at grid 13.5 for the
        // 1px groove beside the index finger.
        rect(
            ctx.g(13.5),
            ctx.g(9.0),
            ctx.g(18.0 - shrink),
            ctx.g(13.5),
            r,
        ),
        // Palm.
        rect(
            ctx.g(7.5),
            ctx.g(12.0),
            ctx.g(18.0 - shrink),
            ctx.g(18.75),
            r,
        ),
        // Thumb nub on the left.
        rect(ctx.g(5.25), ctx.g(11.25), ctx.g(8.25), ctx.g(15.75), r),
    ];
    // Cuff: a 3-grid-unit orange band across the wrist end (a thinner bar
    // leaves no flat core inside its own hairlines, so it would render all
    // blend and read grey rather than orange).
    let cuff = vec![rect(ctx.g(9.0), ctx.g(18.0), ctx.g(16.5), ctx.g(21.0), 0.0)];
    (body, cuff, None)
}

/// The open grab hand: four finger bars over a palm, thumb on the left. No
/// accent of its own (the sheet shows it plain); the closed fist carries the
/// orange knuckles instead, which is what tells the two apart at a glance.
/// A 1px groove separates the bars (F4): narrower bars on a wider spacing
/// than round 4's 0.5px valleys, so they read as fingers, not a mitten.
pub(crate) fn grab(ctx: &Ctx) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    let r = (0.6 * ctx.u).max(0.2);
    let shrink = if ctx.size < 16 { 0.375 } else { 0.0 };
    // F3: at 12px the thumb nub plus its rim touched column 0; tuck it
    // inside below 16px.
    let thumb_x0 = if ctx.size < 16 { 4.5 } else { 3.75 };
    let mut body = vec![
        // Palm.
        rect(
            ctx.g(6.0),
            ctx.g(12.0),
            ctx.g(18.0 - shrink),
            ctx.g(19.5),
            r,
        ),
        // Thumb nub on the left.
        rect(ctx.g(thumb_x0), ctx.g(12.0), ctx.g(6.75), ctx.g(16.5), r),
    ];
    // Four finger bars, each ~1.9 grid units wide on a ~3.4-unit pitch at
    // 24px: ~1px valleys survive the rim as notches, which is what reads as
    // fingers rather than a mitten.
    for (i, fx) in [6.0, 9.375, 12.75, 16.125].into_iter().enumerate() {
        let top = if i == 1 { 6.0 } else { 6.75 };
        body.push(rect(
            ctx.g(fx),
            ctx.g(top),
            ctx.g(fx + 1.875),
            ctx.g(12.75),
            r,
        ));
    }
    (body, Vec::new(), None)
}

/// The grabbing fist: a rounded block with a thumb wrap and orange knuckle
/// ticks across the top. Biased up one row: the thumb mass sits low, so a
/// centred block centroids a pixel and a half too low (caught by the
/// centroid test).
pub(crate) fn grabbing(ctx: &Ctx) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    let r = (1.35 * ctx.u).max(0.3);
    // F3: at 12px the rim blends touched the edge; pull the outer extents
    // slightly inside there.
    let shrink = if ctx.size < 12 {
        0.75
    } else if ctx.size < 16 {
        0.375
    } else {
        0.0
    };
    // F3: at 8/12px the thumb capsule plus its rim touched column 0;
    // tuck it inside below 16px.
    let thumb_dx = if ctx.size < 16 { 1.125 } else { 0.0 };
    let body = vec![
        // Fist block.
        rect(ctx.g(6.0), ctx.g(6.0), ctx.g(18.0 - shrink), ctx.g(16.5), r),
        // Thumb wrap: diagonal bar across the lower left.
        Prim::Capsule {
            ax: ctx.g(5.25 + thumb_dx),
            ay: ctx.g(12.75),
            bx: ctx.g(11.25),
            by: ctx.g(15.75),
            w: (2.7 * ctx.u).max(1.5),
        },
    ];
    // Knuckles: three short orange ticks across the top of the fist, sunk
    // into its face (flush with the top edge they drown in the rim blends).
    let mut knuckles = Vec::with_capacity(3);
    for kx in [7.875, 11.25, 14.625] {
        knuckles.push(Prim::Capsule {
            ax: ctx.g(kx),
            ay: ctx.g(8.25),
            bx: ctx.g(kx),
            by: ctx.g(11.25),
            w: (1.875 * ctx.u).max(1.0),
        });
    }
    (body, knuckles, None)
}
