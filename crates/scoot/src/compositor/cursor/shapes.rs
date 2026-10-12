//! The procedurally-drawn cursor shapes, and which named shape each
//! [`CursorIcon`] maps onto.
//!
//! # Why these are drawn rather than loaded
//!
//! Same reason `cursor.rs`'s arrow is: every pixel here is generated from
//! this module's own vector geometry, so nothing in the tree is a copy of
//! an xcursor theme's asset. niri's cursor assets are GPL and Adwaita's are
//! not MIT-clean (see `CLAUDE.md`'s license note), and this project has no
//! MIT-clean theme to load -- so a compositor that wants to answer
//! `wp_cursor_shape_device_v1.set_shape` with anything other than one
//! shape for every request has to draw the shapes itself. That is what this
//! module is: original line art designed here, not a theme.
//!
//! It is also why the set is deliberately small. Ten shapes cover what a
//! session actually distinguishes by feel -- an I-beam over text, a resize
//! arrow on an edge, a crosshair, a "no" -- and every [`CursorIcon`] outside
//! that set falls back to [`Shape::Arrow`] rather than getting a
//! hand-drawn near-duplicate nobody can tell apart at 16 pixels.
//!
//! # How a shape is built
//!
//! Each shape is described as a signed-distance field over a `size x size`
//! grid -- capsules for strokes (round caps by construction), polygons for
//! the arrow body and arrowheads, a ring for the "no" sign -- and
//! [`raster`](super::raster) evaluates that field at 4x4 subpixel taps per
//! pixel, once per shape at startup or on config reload. The outline sits
//! *outside* the fill everywhere (a band `outline_width` pixels wide), and
//! the arrow alone carries a soft offset shadow; the line-art shapes do not
//! (a blur on a 1.6px stroke reads as blur, not depth, and would break
//! their symmetry about the bitmap middle).
//!
//! Every dimension is a fraction of `size`, clamped into insets that leave
//! room for the outline (and the arrow's shadow) plus one pixel of
//! anti-aliasing bleed, so no fill pixel ever touches the bitmap edge at
//! any allowed size: there is always an outline pixel between the fill and
//! whatever is under the cursor, which is the only thing the outline is
//! for. At `Appearance::MIN_CURSOR_SIZE` (4) the spans collapse and every
//! shape degrades to an honest soft dot rather than a legible drawing;
//! nothing here pretends otherwise, and the tests assert the insets hold at
//! every size a config can ask for.
//!
//! All of this runs exactly once per shape, at startup, from
//! [`Cursor::new`](super::Cursor::new) -- never per frame and never per
//! `set_shape` request, which is an array index into buffers built here.

use smithay::input::pointer::CursorIcon;
use smithay::utils::{Logical, Point};

use super::raster::{self, Shadow};

#[cfg(test)]
mod tests;

/// One drawable cursor shape.
///
/// Not a mirror of [`CursorIcon`] -- see [`Shape::for_icon`] for the mapping
/// and this module's doc for why the set is this small.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// The pointer itself, and the answer for every icon this module does
    /// not draw something specific for.
    Arrow,
    /// An I-beam, for text entry.
    Text,
    /// The I-beam turned on its side, for vertical text.
    VerticalText,
    /// A plus, for precise picking.
    Crosshair,
    /// A vertical double-headed arrow.
    ResizeNs,
    /// A horizontal double-headed arrow.
    ResizeEw,
    /// A double-headed arrow from bottom-left to top-right.
    ResizeNesw,
    /// A double-headed arrow from top-left to bottom-right.
    ResizeNwse,
    /// A four-headed arrow: move, grab, scroll in any direction.
    Move,
    /// A circle with a slash through it.
    NotAllowed,
}

impl Shape {
    /// Every shape, in the order [`Shape::index`] numbers them.
    ///
    /// The one place that order is written down -- [`Cursor`](super::Cursor)
    /// builds its buffers by walking this, and reads them back by
    /// [`Shape::index`], so the two cannot drift apart.
    pub const ALL: [Shape; Self::COUNT] = [
        Shape::Arrow,
        Shape::Text,
        Shape::VerticalText,
        Shape::Crosshair,
        Shape::ResizeNs,
        Shape::ResizeEw,
        Shape::ResizeNesw,
        Shape::ResizeNwse,
        Shape::Move,
        Shape::NotAllowed,
    ];

    /// How many shapes there are, i.e. how many bitmaps
    /// [`Cursor`](super::Cursor) holds.
    pub const COUNT: usize = 10;

    /// This shape's slot in [`Shape::ALL`].
    pub fn index(self) -> usize {
        match self {
            Shape::Arrow => 0,
            Shape::Text => 1,
            Shape::VerticalText => 2,
            Shape::Crosshair => 3,
            Shape::ResizeNs => 4,
            Shape::ResizeEw => 5,
            Shape::ResizeNesw => 6,
            Shape::ResizeNwse => 7,
            Shape::Move => 8,
            Shape::NotAllowed => 9,
        }
    }

    /// The shape drawn for a client's requested [`CursorIcon`].
    ///
    /// Total by construction -- a `_` arm, not an enumeration of the rest:
    /// [`CursorIcon`] is a non-exhaustive upstream enum, so a version bump
    /// that adds an icon must keep compiling and must show *something*
    /// rather than nothing. [`Shape::Arrow`] is that something.
    ///
    /// The groupings are the ones a user can actually distinguish:
    /// `NResize`/`SResize`/`NsResize`/`RowResize` all mean "this edge moves
    /// up and down", and drawing one arrow with a head at each end says that
    /// more clearly than half an arrow would. `Grab`/`Grabbing`/`AllScroll`
    /// share [`Shape::Move`] for the same reason -- an open and a closed hand
    /// are not distinguishable as line art at 16 pixels, and both mean "this
    /// thing is being dragged".
    pub fn for_icon(icon: CursorIcon) -> Shape {
        match icon {
            CursorIcon::Text => Shape::Text,
            CursorIcon::VerticalText => Shape::VerticalText,
            CursorIcon::Crosshair | CursorIcon::Cell => Shape::Crosshair,
            CursorIcon::NResize
            | CursorIcon::SResize
            | CursorIcon::NsResize
            | CursorIcon::RowResize => Shape::ResizeNs,
            CursorIcon::EResize
            | CursorIcon::WResize
            | CursorIcon::EwResize
            | CursorIcon::ColResize => Shape::ResizeEw,
            CursorIcon::NeResize | CursorIcon::SwResize | CursorIcon::NeswResize => {
                Shape::ResizeNesw
            }
            CursorIcon::NwResize | CursorIcon::SeResize | CursorIcon::NwseResize => {
                Shape::ResizeNwse
            }
            CursorIcon::Move
            | CursorIcon::AllScroll
            | CursorIcon::AllResize
            | CursorIcon::Grab
            | CursorIcon::Grabbing => Shape::Move,
            CursorIcon::NotAllowed | CursorIcon::NoDrop => Shape::NotAllowed,
            _ => Shape::Arrow,
        }
    }

    /// The point within this shape's bitmap that lines up with the pointer's
    /// actual location.
    ///
    /// The arrow points with its own tip, which sits `inset` pixels inside
    /// the bitmap's top-left corner (room for the outline and shadow -- see
    /// [`arrow_tip`]). Every other shape here is symmetric about the pointer
    /// and means "the thing under the middle of me", so its hotspot is the
    /// middle pixel's corner -- an I-beam anchored at a bitmap corner would
    /// put the text caret half a cursor away from where the user is
    /// pointing. The strokes themselves sit half a pixel over, on whole
    /// pixels (see `field`'s `c`); that offset is invisible and keeps them
    /// crisp.
    ///
    /// `size` is the bitmap's edge length, already clamped by
    /// [`Cursor::new`](super::Cursor::new).
    pub fn hotspot(self, size: i32) -> Point<i32, Logical> {
        match self {
            Shape::Arrow => {
                let (x, y) = arrow_tip(size);
                (x, y).into()
            }
            _ => (size / 2, size / 2).into(),
        }
    }
}

/// How far past the fill edge the outline reaches, in pixels: 1px at the
/// default size, growing slightly with the bitmap so a large cursor keeps a
/// visible rim, capped so it never becomes a border.
fn outline_width(size: i32) -> f64 {
    (f64::from(size) / 16.0).clamp(1.0, 2.5)
}

/// How wide every line-art stroke is drawn, in pixels: 1.6px at the default
/// size so a stroke survives its own outline on both sides, growing from
/// there so a large cursor is a larger drawing rather than the same hairline
/// on a bigger grid.
fn stroke_width(size: i32) -> f64 {
    (f64::from(size) / 10.0).max(1.6)
}

/// The arrow tip's position in the bitmap: `ceil(outline + 1)` in both axes,
/// room for the outline band plus one pixel of anti-aliasing bleed. The
/// bitmap's top-left corner is transparent padding, never fill -- which is
/// why the hotspot is the tip, not `(0, 0)`.
fn arrow_tip(size: i32) -> (i32, i32) {
    let tip = (outline_width(size) + 1.0).ceil() as i32;
    (tip.max(1), tip.max(1))
}

/// The arrow's bottom-right inset: outline plus the shadow's offset and
/// feather plus bleed. Small bitmaps carry no shadow (it would not fit), so
/// they only reserve the outline there.
fn arrow_pad_br(size: i32) -> i32 {
    let ow = outline_width(size);
    if size < 12 {
        (ow + 1.0).ceil() as i32
    } else {
        let k = (f64::from(size) / 16.0).clamp(1.0, 2.0);
        (ow + 1.0 * k + 1.0 * k + 1.0).ceil() as i32
    }
}

/// The arrow's soft shadow, or `None` on bitmaps too small to hold one.
/// Peak alpha is deliberately low (28% of the fill's own alpha): present on
/// a light background, quiet on a dark one, never a second silhouette.
fn arrow_shadow(size: i32, alpha: f64) -> Option<Shadow> {
    if size < 12 || alpha <= 0.0 {
        return None;
    }
    let k = (f64::from(size) / 16.0).clamp(1.0, 2.0);
    Some(Shadow {
        dx: 0.75 * k,
        dy: 1.0 * k,
        feather: 1.0 * k,
        alpha: 0.28 * alpha,
    })
}

/// The near/far ends of every line-art stroke: mirrored about the hotspot
/// (`size / 2`) and `inset` pixels inside the bitmap edge (outline plus half
/// the stroke plus bleed). Both land on the half-pixel grid -- the middle of
/// a pixel column, not the crack between two -- so a stroke covers whole
/// pixels instead of straddling two columns greyly. When the bitmap is too
/// small for that, both collapse onto the hotspot and the shape degrades to
/// a dot -- the honest outcome at 4px.
fn span(size: i32, inset: f64) -> (f64, f64) {
    let near = inset + 0.5;
    let far = f64::from(size) - inset - 0.5;
    if far <= near {
        let h = f64::from(size / 2) + 0.5;
        (h, h)
    } else {
        (near, far)
    }
}

/// One vector primitive making up a shape's fill.
enum Prim {
    /// A round-capped stroke from `a` to `b`, `w` pixels wide.
    Capsule {
        ax: f64,
        ay: f64,
        bx: f64,
        by: f64,
        w: f64,
    },
    /// A filled polygon (the arrow body, an arrowhead).
    Poly(Vec<(f64, f64)>),
    /// A ring around `(cx, cy)` of radius `r`, `w` pixels wide.
    Ring { cx: f64, cy: f64, r: f64, w: f64 },
    /// A filled disk (only the tiny-size fallback uses this).
    Disk { cx: f64, cy: f64, r: f64 },
}

impl Prim {
    fn distance(&self, px: f64, py: f64) -> f64 {
        match self {
            Prim::Capsule { ax, ay, bx, by, w } => {
                raster::sd_segment(px, py, *ax, *ay, *bx, *by) - w / 2.0
            }
            Prim::Poly(verts) => raster::sd_poly(px, py, verts),
            Prim::Ring { cx, cy, r, w } => {
                (raster::sd_circle(px, py, *cx, *cy, *r)).abs() - w / 2.0
            }
            Prim::Disk { cx, cy, r } => raster::sd_circle(px, py, *cx, *cy, *r),
        }
    }

    /// The tight bounding box of the fill: `(min_x, min_y, max_x, max_y)`.
    fn bounds(&self) -> [f64; 4] {
        match self {
            Prim::Capsule { ax, ay, bx, by, w } => {
                let r = w / 2.0;
                [
                    ax.min(*bx) - r,
                    ay.min(*by) - r,
                    ax.max(*bx) + r,
                    ay.max(*by) + r,
                ]
            }
            Prim::Poly(verts) => {
                let mut b = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for (x, y) in verts {
                    b[0] = b[0].min(*x);
                    b[1] = b[1].min(*y);
                    b[2] = b[2].max(*x);
                    b[3] = b[3].max(*y);
                }
                b
            }
            Prim::Ring { cx, cy, r, w } => {
                let e = r + w / 2.0;
                [cx - e, cy - e, cx + e, cy + e]
            }
            Prim::Disk { cx, cy, r } => [cx - r, cy - r, cx + r, cy + r],
        }
    }
}

/// Rounds the convex corners of `verts` with a chamfer of `r` pixels: each
/// sharp convex vertex becomes a short flat edge, which reads as a rounded
/// corner once supersampled. Concave corners (the arrow's notch) stay sharp.
/// The `tip` vertex -- the arrow's own point, or an arrowhead's -- is only
/// ever chamfered by up to 1px: it is the click point, so it must stay where
/// the hotspot says it is no matter how large the bitmap gets.
/// `r` is clamped to 40% of either incident edge so tiny polygons cannot
/// invert.
fn chamfer(verts: &[(f64, f64)], r: f64, tip: Option<usize>) -> Vec<(f64, f64)> {
    if verts.len() < 3 || r <= 0.0 {
        return verts.to_vec();
    }
    let area: f64 = verts
        .iter()
        .enumerate()
        .map(|(i, (x, y))| {
            let (nx, ny) = verts[(i + 1) % verts.len()];
            x * ny - nx * y
        })
        .sum();
    let mut out = Vec::with_capacity(verts.len() + 4);
    for i in 0..verts.len() {
        let (px, py) = verts[(i + verts.len() - 1) % verts.len()];
        let (vx, vy) = verts[i];
        let (nx, ny) = verts[(i + 1) % verts.len()];
        let cross = (vx - px) * (ny - vy) - (vy - py) * (nx - vx);
        // Convex when the turn matches the winding.
        if cross * area <= 0.0 {
            out.push((vx, vy));
            continue;
        }
        let len_prev = ((vx - px) * (vx - px) + (vy - py) * (vy - py)).sqrt();
        let len_next = ((nx - vx) * (nx - vx) + (ny - vy) * (ny - vy)).sqrt();
        if len_prev <= 0.0 || len_next <= 0.0 {
            out.push((vx, vy));
            continue;
        }
        let mut corner = r;
        if tip == Some(i) {
            corner = corner.min(1.0);
        }
        let cut = corner.min(0.4 * len_prev).min(0.4 * len_next);
        out.push((
            vx + (px - vx) / len_prev * cut,
            vy + (py - vy) / len_prev * cut,
        ));
        out.push((
            vx + (nx - vx) / len_next * cut,
            vy + (ny - vy) / len_next * cut,
        ));
    }
    out
}

/// Draws `shape` into a fresh `size * size * 4` byte `Argb8888` buffer.
///
/// `fill` and `outline` are single premultiplied little-endian `Argb8888`
/// pixels (`[B, G, R, A]`), the same layout `cursor.rs` has always used and
/// the same one the render path scans out. Every shape -- the arrow
/// included -- renders through this function, so the whole set is one
/// family: the same outline width, the same supersampling, the same
/// premultiplication.
pub fn generate(shape: Shape, size: i32, fill: [u8; 4], outline: [u8; 4]) -> Vec<u8> {
    let ow = outline_width(size);
    let alpha = f64::from(fill[3]) / 255.0;
    let (prims, shadow) = field(shape, size, alpha);
    // The rasterizer's skip box: the union of the primitives, expanded past
    // the outline, the shadow and one pixel of bleed. Taps outside it are
    // transparent by construction, so skipping the field there changes
    // nothing and keeps large bitmaps cheap.
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for prim in &prims {
        let b = prim.bounds();
        bounds[0] = bounds[0].min(b[0]);
        bounds[1] = bounds[1].min(b[1]);
        bounds[2] = bounds[2].max(b[2]);
        bounds[3] = bounds[3].max(b[3]);
    }
    let pad = ow + 1.0;
    bounds[0] -= pad;
    bounds[1] -= pad;
    bounds[2] += pad;
    bounds[3] += pad;
    if let Some(sh) = shadow {
        // The shadow samples the field up-left of the tap, so it paints
        // down-right of the fill.
        bounds[2] += sh.dx + sh.feather;
        bounds[3] += sh.dy + sh.feather;
    }
    let field = move |px: f64, py: f64| {
        prims
            .iter()
            .map(|p| p.distance(px, py))
            .fold(f64::INFINITY, raster::union)
    };
    raster::render(size, &field, fill, outline, ow, shadow, Some(bounds))
}

/// Builds the shape's primitives and its shadow (which is `Some` only for
/// [`Shape::Arrow`] on bitmaps large enough to hold one).
fn field(shape: Shape, size: i32, alpha: f64) -> (Vec<Prim>, Option<Shadow>) {
    let ow = outline_width(size);
    let w = stroke_width(size);
    // Strokes sit on the half-pixel grid (the middle of a pixel, e.g. 8.5 in
    // a 16px bitmap) so they cover whole pixels; an integer centerline would
    // straddle two columns and paint both grey. The drawing is centered on
    // the hotspot, half a pixel off the bitmap's own middle on even sizes --
    // invisible, and what `the_balanced_shapes_are_centred_on_the_bitmap`
    // allows for.
    let c = f64::from(size / 2) + 0.5;
    // Corner rounding, growing with the bitmap: subpixel at 16px, a real
    // radius once there are pixels to spend on it.
    let round = (f64::from(size) / 20.0).max(0.8);
    match shape {
        Shape::Arrow => arrow(size, round, alpha),
        _ => {
            let inset = (ow + w / 2.0 + 1.0).ceil();
            let (near, far) = span(size, inset);
            // Collapsed span: an honest dot rather than a panicking stroke.
            if near == far {
                return (
                    vec![Prim::Disk {
                        cx: c,
                        cy: c,
                        r: w / 2.0,
                    }],
                    None,
                );
            }
            let serif = (f64::from(size) / 8.0).max(1.5);
            // Arrowhead reach and half-width, clamped to the span so the
            // heads cannot outgrow their own shaft at any size. The barbs
            // are deliberately wide (a fifth of the bitmap) and short: wide
            // keeps a visible pinch between the two heads instead of
            // merging with the shaft's own outline into a blob, short keeps
            // the heads apart enough that the waist between them survives.
            let head = (f64::from(size) / 5.0).max(3.0).min((far - near) / 2.0);
            let half_base = (f64::from(size) / 5.0).max(2.0).min(c - near).min(far - c);
            let prims = match shape {
                Shape::Arrow => unreachable!("handled above"),
                Shape::Text => vec![
                    Prim::Capsule {
                        ax: c,
                        ay: near,
                        bx: c,
                        by: far,
                        w,
                    },
                    Prim::Capsule {
                        ax: c - serif,
                        ay: near,
                        bx: c + serif,
                        by: near,
                        w,
                    },
                    Prim::Capsule {
                        ax: c - serif,
                        ay: far,
                        bx: c + serif,
                        by: far,
                        w,
                    },
                ],
                Shape::VerticalText => vec![
                    Prim::Capsule {
                        ax: near,
                        ay: c,
                        bx: far,
                        by: c,
                        w,
                    },
                    Prim::Capsule {
                        ax: near,
                        ay: c - serif,
                        bx: near,
                        by: c + serif,
                        w,
                    },
                    Prim::Capsule {
                        ax: far,
                        ay: c - serif,
                        bx: far,
                        by: c + serif,
                        w,
                    },
                ],
                Shape::Crosshair => vec![
                    Prim::Capsule {
                        ax: c,
                        ay: near,
                        bx: c,
                        by: far,
                        w,
                    },
                    Prim::Capsule {
                        ax: near,
                        ay: c,
                        bx: far,
                        by: c,
                        w,
                    },
                ],
                Shape::ResizeNs => {
                    let mut v = vec![Prim::Capsule {
                        ax: c,
                        ay: near,
                        bx: c,
                        by: far,
                        w,
                    }];
                    v.push(Prim::Poly(chamfer(
                        &[
                            (c, near),
                            (c - half_base, near + head),
                            (c + half_base, near + head),
                        ],
                        round * 0.5,
                        Some(0),
                    )));
                    v.push(Prim::Poly(chamfer(
                        &[
                            (c, far),
                            (c + half_base, far - head),
                            (c - half_base, far - head),
                        ],
                        round * 0.5,
                        Some(0),
                    )));
                    v
                }
                Shape::ResizeEw => {
                    let mut v = vec![Prim::Capsule {
                        ax: near,
                        ay: c,
                        bx: far,
                        by: c,
                        w,
                    }];
                    v.push(Prim::Poly(chamfer(
                        &[
                            (near, c),
                            (near + head, c - half_base),
                            (near + head, c + half_base),
                        ],
                        round * 0.5,
                        Some(0),
                    )));
                    v.push(Prim::Poly(chamfer(
                        &[
                            (far, c),
                            (far - head, c + half_base),
                            (far - head, c - half_base),
                        ],
                        round * 0.5,
                        Some(0),
                    )));
                    v
                }
                Shape::ResizeNwse => {
                    let mut v = vec![Prim::Capsule {
                        ax: near,
                        ay: near,
                        bx: far,
                        by: far,
                        w,
                    }];
                    v.push(Prim::Poly(chamfer(
                        &[(near, near), (near + head, near), (near, near + head)],
                        round * 0.5,
                        Some(0),
                    )));
                    v.push(Prim::Poly(chamfer(
                        &[(far, far), (far, far - head), (far - head, far)],
                        round * 0.5,
                        Some(0),
                    )));
                    v
                }
                Shape::ResizeNesw => {
                    let mut v = vec![Prim::Capsule {
                        ax: far,
                        ay: near,
                        bx: near,
                        by: far,
                        w,
                    }];
                    v.push(Prim::Poly(chamfer(
                        &[(far, near), (far, near + head), (far - head, near)],
                        round * 0.5,
                        Some(0),
                    )));
                    v.push(Prim::Poly(chamfer(
                        &[(near, far), (near, far - head), (near + head, far)],
                        round * 0.5,
                        Some(0),
                    )));
                    v
                }
                Shape::Move => {
                    let mut v = vec![
                        Prim::Capsule {
                            ax: c,
                            ay: near,
                            bx: c,
                            by: far,
                            w,
                        },
                        Prim::Capsule {
                            ax: near,
                            ay: c,
                            bx: far,
                            by: c,
                            w,
                        },
                    ];
                    v.push(Prim::Poly(chamfer(
                        &[
                            (c, near),
                            (c - half_base, near + head),
                            (c + half_base, near + head),
                        ],
                        round * 0.5,
                        Some(0),
                    )));
                    v.push(Prim::Poly(chamfer(
                        &[
                            (c, far),
                            (c + half_base, far - head),
                            (c - half_base, far - head),
                        ],
                        round * 0.5,
                        Some(0),
                    )));
                    v.push(Prim::Poly(chamfer(
                        &[
                            (near, c),
                            (near + head, c - half_base),
                            (near + head, c + half_base),
                        ],
                        round * 0.5,
                        Some(0),
                    )));
                    v.push(Prim::Poly(chamfer(
                        &[
                            (far, c),
                            (far - head, c + half_base),
                            (far - head, c - half_base),
                        ],
                        round * 0.5,
                        Some(0),
                    )));
                    v
                }
                Shape::NotAllowed => {
                    let radius = far - c;
                    if radius < 1.0 {
                        vec![Prim::Disk {
                            cx: c,
                            cy: c,
                            r: w / 2.0,
                        }]
                    } else {
                        let reach = radius * 7.0 / 10.0;
                        vec![
                            Prim::Ring {
                                cx: c,
                                cy: c,
                                r: radius,
                                w,
                            },
                            Prim::Capsule {
                                ax: c - reach,
                                ay: c + reach,
                                bx: c + reach,
                                by: c - reach,
                                w,
                            },
                        ]
                    }
                }
            };
            (prims, None)
        }
    }
}

/// The arrow body: an original six-vertex silhouette pointing north-west --
/// a vertical left edge, a concave notch, and a tail running down-right --
/// chamfered at its convex corners and inset for outline, shadow and bleed.
///
/// Fractions are of the drawable box (`size` minus the insets), so the
/// shape at 4px and at 256px is the same drawing, not the same pixel count.
fn arrow(size: i32, round: f64, alpha: f64) -> (Vec<Prim>, Option<Shadow>) {
    let (tx, ty) = arrow_tip(size);
    let br = arrow_pad_br(size);
    // A bitmap too small for the insets gets an honest dot, not a polygon
    // clipped to slivers: the same fallback the line-art shapes use.
    if size - tx - br < 1 {
        let c = f64::from(size) / 2.0;
        return (
            vec![Prim::Disk {
                cx: c,
                cy: c,
                r: 0.75,
            }],
            None,
        );
    }
    let bw = f64::from(size - tx - br);
    let bh = f64::from(size - ty - br);
    let ox = f64::from(tx);
    let oy = f64::from(ty);
    // Six vertices, tip first, clockwise: tip, left-edge foot, notch, tail
    // left, tail right, barb, and back to the tip along the long diagonal.
    // The tail's two long edges (notch->tail-left and barb->tail-right) are
    // parallel by construction: barb = tail-right - (tail-left - notch).
    let notch = (0.20 * bw, 0.54 * bh);
    let tail_left = (0.30 * bw, 0.84 * bh);
    let tail_right = (0.50 * bw, 0.78 * bh);
    let barb = (
        tail_right.0 - (tail_left.0 - notch.0),
        tail_right.1 - (tail_left.1 - notch.1),
    );
    let poly = vec![
        (0.0, 0.0),
        (0.0, 0.68 * bh),
        notch,
        tail_left,
        tail_right,
        barb,
    ]
    .into_iter()
    .map(|(x, y)| (ox + x, oy + y))
    .collect::<Vec<_>>();
    (
        vec![Prim::Poly(chamfer(&poly, round, Some(0)))],
        arrow_shadow(size, alpha),
    )
}
