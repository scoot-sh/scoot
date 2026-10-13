//! The text beams: the I-beam and its sideways sibling.
//!
//! Authored on the 24-unit grid ([`super::design`]): the serif reach below
//! is 3 grid units, the same 2px at 16px the beam has always used.
//!
//! The stem and both serifs are the body; the accent is a thin mark down the
//! stem's middle (the orange bar the sheet's `text-select` cell shows). With
//! the accent disabled the beam is the plain I-beam of the sheet's `text`
//! cell, so both readings survive one geometry.

use super::{Ctx, Prim, span};

/// Shared I-beam builder along one axis: `vertical` draws the `text` beam,
/// horizontal the `vertical-text` one.
fn beam(ctx: &Ctx, vertical: bool) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    let mut inset = (ctx.ow + ctx.w / 2.0 + 1.0).ceil();
    // Below 12px the serif end caps plus the rim land on the last column
    // (Text at 8px put flat rim on column 7): reserve one more pixel and
    // shorten the serifs so the whole beam keeps its rim inside.
    let mut serif_scale = 1.0;
    if ctx.size < 12 {
        inset += 1.0;
        serif_scale = 0.7;
    } else if ctx.size < 16 {
        inset += 0.5;
        serif_scale = 0.85;
    }
    let (near, far) = span(ctx.size, inset);
    // 3 grid units: the same reach as the old `size / 8`.
    let serif = (3.0 * f64::from(ctx.size) / super::design::GRID).max(1.5) * serif_scale;
    let c = ctx.c;
    let (stem, bars, mark) = if vertical {
        (
            Prim::Capsule {
                ax: c,
                ay: near,
                bx: c,
                by: far,
                w: ctx.w,
            },
            vec![
                Prim::Capsule {
                    ax: c - serif,
                    ay: near,
                    bx: c + serif,
                    by: near,
                    w: ctx.w,
                },
                Prim::Capsule {
                    ax: c - serif,
                    ay: far,
                    bx: c + serif,
                    by: far,
                    w: ctx.w,
                },
            ],
            Prim::Capsule {
                ax: c,
                ay: near + (far - near) * 0.22,
                bx: c,
                by: far - (far - near) * 0.22,
                w: (ctx.w * 0.42).max(0.8),
            },
        )
    } else {
        (
            Prim::Capsule {
                ax: near,
                ay: c,
                bx: far,
                by: c,
                w: ctx.w,
            },
            vec![
                Prim::Capsule {
                    ax: near,
                    ay: c - serif,
                    bx: near,
                    by: c + serif,
                    w: ctx.w,
                },
                Prim::Capsule {
                    ax: far,
                    ay: c - serif,
                    bx: far,
                    by: c + serif,
                    w: ctx.w,
                },
            ],
            Prim::Capsule {
                ax: near + (far - near) * 0.22,
                ay: c,
                bx: far - (far - near) * 0.22,
                by: c,
                w: (ctx.w * 0.42).max(0.8),
            },
        )
    };
    let mut body = vec![stem];
    body.extend(bars);
    (body, vec![mark], None)
}

/// The I-beam for text entry.
pub(crate) fn text(ctx: &Ctx) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    beam(ctx, true)
}

/// The I-beam turned on its side, for vertical text.
pub(crate) fn vertical_text(
    ctx: &Ctx,
) -> (Vec<Prim>, Vec<Prim>, Option<super::super::raster::Shadow>) {
    beam(ctx, false)
}
