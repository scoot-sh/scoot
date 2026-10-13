//! Turns the Mocu art ([`super::mocu`]) into fill primitives at one bitmap
//! size: where each outline sits in the bitmap, how big it is drawn, where
//! its hotspot lands, and which part of it wears the ginger accent.
//!
//! # Placement
//!
//! Mocu is drawn on a 24-unit grid whose outlines reach the bitmap edge once
//! their 1-unit rim is added. scoot's rim keeps its own width (see
//! [`outline_width`]) and must stay clear of the bitmap edge (the rim is what
//! separates the cursor from whatever it is over), so each shape is mapped
//! into the bitmap around its Mocu hotspot:
//!
//! - **Scale** `k` is `size / 24` -- Mocu's own proportions -- unless that
//!   would push the rim off the bitmap, in which case the shape shrinks
//!   just enough to keep a clear pixel past its rim on every side.
//! - **Anchor**: the hotspot lands on a fixed bitmap position. Pointing
//!   shapes (the arrow family, the hands, the magnifier) put it as close to
//!   the top-left as the rim allows; the symmetric shapes put it at the
//!   bitmap's middle -- the middle pixel's centre for art whose hotspot is a
//!   pixel centre in Mocu (`11.5`), the middle pixel's corner for art whose
//!   hotspot is a grid point (`12`). [`Placement::hotspot`] is the pixel the
//!   anchor falls in, so the click point and the drawing cannot disagree.
//!
//! # Grid fitting
//!
//! Mocu's art is pixel-exact at 24px, where one unit is one pixel. At 16px
//! a unit is two thirds of a pixel, so a plain scale would leave every
//! straight edge straddling two pixels and every one-unit feature (the
//! I-beam's stem, the gaps between the hand's fingers) as a grey smear. So
//! each axis is fitted to the pixel grid the way a font hinter fits stems:
//! the coordinates of the art's horizontal and vertical edges (plus its
//! extremes) are *knots*, each knot is rounded to a whole pixel (ties away
//! from the hotspot, so symmetric art stays symmetric), any two knots at
//! least a unit apart stay at least a pixel apart (so no stem or gap ever
//! vanishes), and everything between knots -- diagonals, arcs -- is mapped
//! piecewise-linearly. Where a unit is a whole number of pixels (24px,
//! 48px) every whole-unit edge already lands on the grid and stays exactly
//! where Mocu draws it. From 24px up nothing moves more than rounding (half
//! a pixel) plus, for an extreme tip, up to a pixel given up to keep its rim
//! clear of the bitmap edge. Below 24px the fit is what keeps a one-unit
//! feature a whole crisp pixel instead of a smear.

use super::svgpath::{self, Subpath};
use super::{Ctx, Prim, mocu, outline_width, paw};
use smithay::utils::{Logical, Point};

/// The flattening tolerance, in pixels: curves stay within a tenth of a
/// pixel of the true outline at every size.
const FLATTEN_PX: f64 = 0.1;

/// How a shape's hotspot is anchored in its bitmap.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Anchor {
    /// As far up and left as the rim allows (pointing shapes).
    Corner,
    /// At the bitmap's middle (symmetric shapes).
    Middle,
}

/// One axis of a [`Placement`]: grid-unit knots and the pixel each lands
/// on, ascending in both.
#[derive(Clone, Debug)]
struct Axis {
    knots: Vec<(f64, f64)>,
    /// Pixels per unit, for extrapolating past the outermost knots.
    k: f64,
}

impl Axis {
    /// Pulls every knot inside `lo..=hi` -- a tip that rounding or Mocu's
    /// own reach put past the clearance gives up under a pixel rather than
    /// the whole shape shrinking. Fails (returns `false`) if that would
    /// merge two knots that were a pixel or more apart, or move the
    /// hotspot's own knot: then the shape has to shrink after all.
    fn clamp(&mut self, lo: f64, hi: f64, hot: f64) -> bool {
        let before: Vec<f64> = self.knots.iter().map(|kn| kn.1).collect();
        for kn in &mut self.knots {
            let p = kn.1.clamp(lo, hi);
            if (kn.0 - hot).abs() < 1e-9 && p != kn.1 {
                return false;
            }
            kn.1 = p;
        }
        self.knots
            .windows(2)
            .zip(before.windows(2))
            .all(|(now, was)| was[1] - was[0] < 1.0 || now[1].1 - now[0].1 >= 1.0)
    }

    fn map(&self, u: f64) -> f64 {
        let ks = &self.knots;
        let Some(&(u0, p0)) = ks.first() else {
            return u * self.k;
        };
        if u <= u0 {
            return p0 - (u0 - u) * self.k;
        }
        for w in ks.windows(2) {
            let ((ua, pa), (ub, pb)) = (w[0], w[1]);
            if u <= ub {
                return pa + (u - ua) / (ub - ua) * (pb - pa);
            }
        }
        let &(un, pn) = ks.last().expect("non-empty");
        pn + (u - un) * self.k
    }
}

/// One shape's mapping from Mocu's 24-grid into bitmap pixels.
#[derive(Clone, Debug)]
pub(crate) struct Placement {
    /// Where the Mocu hotspot lands, in bitmap pixels.
    pub ax: f64,
    pub ay: f64,
    x: Axis,
    y: Axis,
}

impl Placement {
    /// A grid point in bitmap pixels.
    pub(crate) fn px(&self, x: f64, y: f64) -> (f64, f64) {
        (self.x.map(x), self.y.map(y))
    }

    /// The hotspot pixel: the one the anchor falls in.
    pub(crate) fn hotspot(&self) -> Point<i32, Logical> {
        (self.ax.floor() as i32, self.ay.floor() as i32).into()
    }

    fn map(&self, subs: &[Subpath]) -> Vec<Subpath> {
        subs.iter()
            .map(|s| s.iter().map(|&(x, y)| self.px(x, y)).collect())
            .collect()
    }
}

/// One coat of a shape: body outlines plus the accent mark drawn over them,
/// and whether the mark wears a rim-colored hairline (a halo for a mark that
/// floats off the body; none for one inlaid in it, where a hairline would
/// only grey the mark at small sizes). A shape is one layer or two: a badge
/// (the wait spinner, the "?") is its own layer composited over the arrow,
/// so its rim separates it from the arrow the way Mocu's stroke does.
pub(crate) struct Layer {
    pub body: Vec<Prim>,
    pub accent: Vec<Prim>,
    pub accent_rim: bool,
}

/// What wears the accent in a layer.
enum Mark {
    None,
    /// The layer's first outline, clipped to this convex polygon (grid units).
    Clip(Vec<(f64, f64)>),
    /// The layer's outline at this index, whole (the magnifier's sign).
    Whole(usize),
    /// A paw print centred at `(x, y)` with radius `r` (grid units) as both
    /// body and accent: the paw badge, which Mocu has no art for.
    Paw(f64, f64, f64),
    /// Mocu's progress spinner frozen: three quarters of its annulus.
    Spinner,
}

struct SrcLayer {
    outlines: Vec<Vec<Subpath>>,
    mark: Mark,
}

/// A shape's art in grid units, flattened, plus where its hotspot goes.
struct Source {
    layers: Vec<SrcLayer>,
    hot: (f64, f64),
    anchor: Anchor,
}

/// The Mocu-sourced shapes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Art {
    Arrow,
    PointerBadge,
    WaitBadge,
    Help,
    Text,
    VerticalText,
    Hand,
    Crosshair,
    Cell,
    ResizeNs,
    ResizeEw,
    ResizeNesw,
    ResizeNwse,
    Move,
    NotAllowed,
    Grab,
    Grabbing,
    SplitV,
    SplitH,
    ZoomIn,
    ZoomOut,
}

fn tolerance(size: i32) -> f64 {
    FLATTEN_PX / (f64::from(size) / 24.0).max(1e-3)
}

fn flatten(d: &str, size: i32) -> Vec<Subpath> {
    svgpath::parse(d, tolerance(size))
}

/// An axis-aligned clip rectangle in grid units.
fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<(f64, f64)> {
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

/// The arrow tail's last stretch, as a clip in grid units: everything past
/// a line across the tail `len` units before its end, perpendicular to the
/// tail's own axis (Mocu's tail runs from the notch `(4.94, 13.12)` to the
/// end edge `(6.66, 17.28)`-`(8.5, 16.5)`).
fn tail_tip(len: f64) -> Vec<(f64, f64)> {
    let (ax, ay): (f64, f64) = (6.66 - 4.94, 17.28 - 13.12);
    let n = (ax * ax + ay * ay).sqrt();
    let (ux, uy) = (ax / n, ay / n);
    let (mx, my) = (
        (6.66 + 8.5) / 2.0 - ux * len,
        (17.28 + 16.5) / 2.0 - uy * len,
    );
    let (px, py) = (-uy, ux);
    let (w, far) = (6.0, 6.0);
    vec![
        (mx + px * w, my + py * w),
        (mx - px * w, my - py * w),
        (mx - px * w + ux * far, my - py * w + uy * far),
        (mx + px * w + ux * far, my + py * w + uy * far),
    ]
}

/// How far back from its end the arrow's tail wears the accent, in units.
const TAIL_MARK: f64 = 3.0;

/// Where the paw badge sits beside the arrow, in grid units: below and right
/// of the tail, clear of the barb.
const PAW_BADGE: (f64, f64, f64) = (16.25, 16.5, 4.0);

/// A paw print's extent around its centre, as multiples of its radius
/// (left/right, top, bottom): the toes reach `0.78 + 0.40` out and
/// `0.70 + 0.40` up, the pad `0.30 + 0.72` down (see [`paw::prims`]).
const PAW_REACH: (f64, f64, f64) = (1.18, 1.10, 1.02);

fn source(art: Art, size: i32) -> Source {
    use mocu::*;
    let layer = |o: &Outline, mark: Mark| SrcLayer {
        outlines: o.paths.iter().map(|d| flatten(d, size)).collect(),
        mark,
    };
    let arrow = || SrcLayer {
        outlines: vec![flatten(DEFAULT.paths[0], size)],
        mark: Mark::Clip(tail_tip(TAIL_MARK)),
    };
    let one = |o: &Outline, anchor: Anchor, mark: Mark| Source {
        layers: vec![layer(o, mark)],
        hot: o.hot,
        anchor,
    };
    let (dx, dy, dr) = PROGRESS_DISK;
    match art {
        Art::Arrow => Source {
            layers: vec![arrow()],
            hot: DEFAULT.hot,
            anchor: Anchor::Corner,
        },
        Art::PointerBadge => {
            let (x, y, r) = PAW_BADGE;
            Source {
                layers: vec![
                    arrow(),
                    SrcLayer {
                        // The paw's extent, so placement and fitting see it.
                        outlines: {
                            let (side, up, down) = PAW_REACH;
                            vec![vec![vec![
                                (x - side * r, y - up * r),
                                (x + side * r, y - up * r),
                                (x + side * r, y + down * r),
                                (x - side * r, y + down * r),
                            ]]]
                        },
                        mark: Mark::Paw(x, y, r),
                    },
                ],
                hot: DEFAULT.hot,
                anchor: Anchor::Corner,
            }
        }
        Art::WaitBadge => Source {
            layers: vec![
                arrow(),
                SrcLayer {
                    outlines: vec![vec![circle(dx, dy, dr, size)]],
                    mark: Mark::Spinner,
                },
            ],
            hot: DEFAULT.hot,
            anchor: Anchor::Corner,
        },
        Art::Help => Source {
            layers: vec![
                arrow(),
                SrcLayer {
                    outlines: vec![flatten(HELP_MARK, size)],
                    mark: Mark::None,
                },
            ],
            hot: DEFAULT.hot,
            anchor: Anchor::Corner,
        },
        Art::Text => one(&TEXT, Anchor::Middle, Mark::None),
        Art::VerticalText => one(&VERTICAL_TEXT, Anchor::Middle, Mark::None),
        // The cuff: the hand's bottom band.
        Art::Hand => one(
            &POINTER,
            Anchor::Corner,
            Mark::Clip(rect(0.0, 15.0, 24.0, 18.0)),
        ),
        Art::Crosshair => one(
            &CROSSHAIR,
            Anchor::Middle,
            Mark::Clip(rect(10.0, 10.0, 13.0, 13.0)),
        ),
        Art::Cell => one(&CELL, Anchor::Middle, Mark::None),
        // The resize arrows wear the accent on their shafts, between the
        // heads.
        Art::ResizeNs => one(
            &NS_RESIZE,
            Anchor::Middle,
            Mark::Clip(rect(9.0, 8.0, 14.0, 15.0)),
        ),
        Art::ResizeEw => one(
            &EW_RESIZE,
            Anchor::Middle,
            Mark::Clip(rect(8.0, 9.0, 15.0, 14.0)),
        ),
        Art::ResizeNesw => one(
            &NESW_RESIZE,
            Anchor::Middle,
            Mark::Clip(vec![(15.0, 10.0), (10.0, 15.0), (8.0, 13.0), (13.0, 8.0)]),
        ),
        Art::ResizeNwse => one(
            &NWSE_RESIZE,
            Anchor::Middle,
            Mark::Clip(vec![(8.0, 10.0), (10.0, 8.0), (15.0, 13.0), (13.0, 15.0)]),
        ),
        // Move: the centre disk.
        Art::Move => one(
            &ALL_SCROLL,
            Anchor::Middle,
            Mark::Clip(rect(7.5, 7.5, 14.5, 14.5)),
        ),
        // Not allowed: the slash between the two half-moon holes.
        Art::NotAllowed => one(
            &NOT_ALLOWED,
            Anchor::Middle,
            Mark::Clip(vec![
                (8.55, 6.79),
                (17.21, 15.45),
                (15.45, 17.21),
                (6.79, 8.55),
            ]),
        ),
        Art::Grab => one(&GRAB, Anchor::Corner, Mark::None),
        Art::Grabbing => one(&GRABBING, Anchor::Corner, Mark::None),
        // The splits wear the accent on their divider bar.
        Art::SplitV => one(
            &COL_RESIZE,
            Anchor::Middle,
            Mark::Clip(rect(9.5, 3.0, 12.5, 21.0)),
        ),
        Art::SplitH => one(
            &ROW_RESIZE,
            Anchor::Middle,
            Mark::Clip(rect(3.0, 9.5, 21.0, 12.5)),
        ),
        Art::ZoomIn => one(&ZOOM_IN, Anchor::Corner, Mark::Whole(1)),
        Art::ZoomOut => one(&ZOOM_OUT, Anchor::Corner, Mark::Whole(1)),
    }
}

/// A full circle as one flattened subpath, in grid units.
fn circle(cx: f64, cy: f64, r: f64, size: i32) -> Subpath {
    let n = arc_steps(r, std::f64::consts::TAU, size).max(8);
    (0..n)
        .map(|i| {
            let t = std::f64::consts::TAU * i as f64 / n as f64;
            (cx + r * t.cos(), cy + r * t.sin())
        })
        .collect()
}

/// Segments for an arc of radius `r` (units) through `sweep` radians so the
/// chord error stays under [`FLATTEN_PX`].
fn arc_steps(r: f64, sweep: f64, size: i32) -> usize {
    let tol = tolerance(size);
    let step = if tol >= r {
        std::f64::consts::FRAC_PI_2
    } else {
        2.0 * (1.0 - tol / r).acos()
    };
    ((sweep.abs() / step.max(1e-3)).ceil() as usize).clamp(2, 512)
}

/// An annular sector (a thick arc) as one subpath, in grid units: from
/// `start` clockwise through `sweep` radians, between radii `r0` and `r1`.
fn arc_band(cx: f64, cy: f64, r0: f64, r1: f64, start: f64, sweep: f64, size: i32) -> Subpath {
    let n = arc_steps(r1, sweep, size);
    let at = |r: f64, i: usize| {
        let t = start + sweep * i as f64 / n as f64;
        (cx + r * t.cos(), cy + r * t.sin())
    };
    let mut out: Subpath = (0..=n).map(|i| at(r1, i)).collect();
    out.extend((0..=n).rev().map(|i| at(r0, i)));
    out
}

/// Clips every subpath of `subs` to the convex polygon `clip` (both in the
/// same space; `clip` in either winding), Sutherland-Hodgman style. Used to
/// cut accent marks out of Mocu's outlines so they follow its edges exactly.
pub(crate) fn clip(subs: &[Subpath], clip: &[(f64, f64)]) -> Vec<Subpath> {
    let area: f64 = (0..clip.len())
        .map(|i| {
            let (ax, ay) = clip[i];
            let (bx, by) = clip[(i + 1) % clip.len()];
            ax * by - bx * ay
        })
        .sum();
    let sign = if area >= 0.0 { 1.0 } else { -1.0 };
    let mut out = Vec::new();
    for sub in subs {
        let mut poly = sub.clone();
        for i in 0..clip.len() {
            if poly.is_empty() {
                break;
            }
            let (ax, ay) = clip[i];
            let (bx, by) = clip[(i + 1) % clip.len()];
            let side = |p: (f64, f64)| sign * ((bx - ax) * (p.1 - ay) - (by - ay) * (p.0 - ax));
            let mut next = Vec::with_capacity(poly.len() + 2);
            for j in 0..poly.len() {
                let p = poly[j];
                let q = poly[(j + 1) % poly.len()];
                let (sp, sq) = (side(p), side(q));
                if sp >= 0.0 {
                    next.push(p);
                }
                if (sp >= 0.0) != (sq >= 0.0) {
                    let t = sp / (sp - sq);
                    next.push((p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t));
                }
            }
            poly = next;
        }
        if poly.len() >= 3 {
            out.push(poly);
        }
    }
    out
}

/// The grid-unit knots of one axis: every coordinate where the art has an
/// edge along the *other* axis (`x` of vertical edges for the x axis) --
/// the edges a pixel grid can make crisp -- plus the art's two extremes on
/// this axis, so its overall extent lands on whole pixels too (a tip or an
/// arc's outermost point moves by at most half a pixel). Diagonals and arcs
/// between knots are not rounded, only carried along. Sorted, deduplicated.
fn knots(src: &Source, vertical: bool) -> Vec<f64> {
    let coord = |p: &(f64, f64)| if vertical { p.0 } else { p.1 };
    let other = |p: &(f64, f64)| if vertical { p.1 } else { p.0 };
    let mut out = Vec::new();
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for sub in src.layers.iter().flat_map(|l| l.outlines.iter()).flatten() {
        for i in 0..sub.len() {
            let a = &sub[i];
            let b = &sub[(i + 1) % sub.len()];
            lo = lo.min(coord(a));
            hi = hi.max(coord(a));
            if (coord(a) - coord(b)).abs() < 1e-9 && (other(a) - other(b)).abs() > 1e-9 {
                out.push(coord(a));
            }
        }
    }
    if lo.is_finite() {
        out.push(lo);
        out.push(hi);
    }
    out.sort_by(f64::total_cmp);
    out.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    out
}

/// The art's bounding box in grid units: `(min_x, min_y, max_x, max_y)`.
fn extent(src: &Source) -> [f64; 4] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for (x, y) in src
        .layers
        .iter()
        .flat_map(|l| l.outlines.iter())
        .flatten()
        .flatten()
    {
        b[0] = b[0].min(*x);
        b[1] = b[1].min(*y);
        b[2] = b[2].max(*x);
        b[3] = b[3].max(*y);
    }
    b
}

/// Fits one axis: `hot` lands on `anchor`, every knot on a whole pixel (ties
/// away from the anchor), knots a unit or more apart at least a pixel apart.
fn fit_axis(knots: &[f64], hot: f64, anchor: f64, k: f64) -> Axis {
    let target = |u: f64| anchor + (u - hot) * k;
    // A real feature (a unit or more) keeps at least a pixel -- unless the
    // bitmap is so small that the feature would be under half a pixel even
    // unfitted (the 8px and 12px sizes), where widening every one would
    // overflow the bitmap and plain rounding is the honest outcome.
    let keeps_apart = |du: f64| du >= 0.99 && du * k >= 0.5;
    let round_away = |p: f64| {
        if p >= anchor {
            (p + 0.5).floor()
        } else {
            (p - 0.5).ceil()
        }
    };
    // The hotspot is pinned to the anchor, but it only holds the next knot
    // a pixel away if it is an edge itself (the arrow's tip column is; the
    // hand's fingertip centre is not, and spacing from it would stretch
    // the finger).
    let hot_is_edge = knots.iter().any(|&u| (u - hot).abs() < 1e-6);
    let mut above: Vec<(f64, f64)> = Vec::new();
    let mut prev = (hot, anchor);
    let mut first = !hot_is_edge;
    for &u in knots.iter().filter(|&&u| u > hot + 1e-6) {
        let mut p = round_away(target(u));
        if std::mem::take(&mut first) {
            p = p.max(prev.1);
        } else if keeps_apart(u - prev.0) {
            p = p.max(prev.1 + 1.0).max((prev.1 + 1.0).ceil());
        } else {
            p = p.max(prev.1);
        }
        above.push((u, p));
        prev = (u, p);
    }
    let mut below: Vec<(f64, f64)> = Vec::new();
    prev = (hot, anchor);
    first = !hot_is_edge;
    for &u in knots.iter().rev().filter(|&&u| u < hot - 1e-6) {
        let mut p = round_away(target(u));
        if std::mem::take(&mut first) {
            p = p.min(prev.1);
        } else if keeps_apart(prev.0 - u) {
            p = p.min(prev.1 - 1.0).min((prev.1 - 1.0).floor());
        } else {
            p = p.min(prev.1);
        }
        below.push((u, p));
        prev = (u, p);
    }
    below.reverse();
    let mut all = below;
    all.push((hot, anchor));
    all.extend(above);
    Axis { knots: all, k }
}

/// Fits `src` into a `size` bitmap; see the module docs.
fn place(src: &Source, size: i32) -> Placement {
    let s = f64::from(size);
    let unit = s / 24.0;
    // Clearance from a fill edge to the bitmap edge: the rim plus one clear
    // pixel (whose subpixel taps then all miss the rim).
    let clear = outline_width(size) + 1.0;
    let xs = knots(src, true);
    let ys = knots(src, false);
    let [lo_x, lo_y, hi_x, hi_y] = extent(src);
    let (hx, hy) = src.hot;
    let frac = |h: f64| h - h.floor();
    // The hotspot's sub-pixel phase. At Mocu's own scale and above, its
    // whole grid points sit on pixel corners and its half points half a
    // unit -- `k / 2` pixels -- past one: a `11.5` hotspot is a pixel centre
    // at 24px but a pixel corner at 48px, so every edge a whole unit from it
    // lands on a pixel edge at both and the fit is exact wherever `k` is
    // whole. Below Mocu's scale nothing lands exactly anyway, and a
    // half-unit hotspot stays on a pixel centre: symmetric art then rounds
    // to odd, centred widths (the 16px resize shaft keeps 3px, not 2).
    let phase = |h: f64, k: f64| {
        if k < 1.0 {
            frac(h)
        } else {
            (frac(h) * k).rem_euclid(1.0)
        }
    };
    let anchor = |h: f64, lo: f64, k: f64| match src.anchor {
        Anchor::Corner => {
            let want = clear + (h - lo) * k;
            (want - phase(h, k)).ceil() + phase(h, k)
        }
        Anchor::Middle => f64::from(size / 2) + phase(h, k),
    };
    let mut k = unit;
    let mut best = None;
    // Shrink until the fitted art keeps its clearance on every side. Grid
    // fitting can widen art by a pixel or two at small sizes, so this checks
    // the fitted extremes rather than the unfitted ones.
    for _ in 0..24 {
        let (mut ax, mut ay) = (anchor(hx, lo_x, k), anchor(hy, lo_y, k));
        let mut x = fit_axis(&xs, hx, ax, k);
        let mut y = fit_axis(&ys, hy, ay, k);
        // A corner anchor moves rather than shrinking the art when fitting
        // rounded its near side past the clearance (by whole pixels, so the
        // fit itself is unchanged).
        if src.anchor == Anchor::Corner {
            let (dx, dy) = (
                (clear - x.map(lo_x)).max(0.0).ceil(),
                (clear - y.map(lo_y)).max(0.0).ceil(),
            );
            if dx > 0.0 {
                ax += dx;
                x = fit_axis(&xs, hx, ax, k);
            }
            if dy > 0.0 {
                ay += dy;
                y = fit_axis(&ys, hy, ay, k);
            }
        }
        // Symmetric art clamps symmetrically about its middle, so a tip
        // given up on one side is given up on the other too.
        let limits = |a: f64| match src.anchor {
            Anchor::Corner => (clear, s - clear),
            Anchor::Middle => {
                let reach = (a - clear).min(s - clear - a);
                (a - reach, a + reach)
            }
        };
        let ((xl, xh), (yl, yh)) = (limits(ax), limits(ay));
        let clamped = x.clamp(xl, xh, hx) && y.clamp(yl, yh, hy);
        let (x0, x1, y0, y1) = (x.map(lo_x), x.map(hi_x), y.map(lo_y), y.map(hi_y));
        let p = Placement { ax, ay, x, y };
        let fits = clamped && x0 >= clear && y0 >= clear && x1 <= s - clear && y1 <= s - clear;
        best = Some(p);
        if fits {
            break;
        }
        k *= 0.94;
    }
    best.expect("at least one round")
}

/// Where `art` lands in a `size` bitmap.
pub(crate) fn placement(art: Art, size: i32) -> Placement {
    place(&source(art, size), size)
}

fn path(subs: Vec<Subpath>) -> Prim {
    Prim::Path(subs)
}

/// Draws `art` at `size`, one [`Layer`] per coat, back to front.
pub(crate) fn draw(art: Art, ctx: &Ctx) -> Vec<Layer> {
    let size = ctx.size;
    let src = source(art, size);
    let p = place(&src, size);
    src.layers
        .iter()
        .map(|layer| {
            let mut body: Vec<Prim> = layer.outlines.iter().map(|o| path(p.map(o))).collect();
            let accent = match &layer.mark {
                Mark::None => Vec::new(),
                Mark::Clip(region) => {
                    let cut = clip(&layer.outlines[0], region);
                    if cut.is_empty() {
                        Vec::new()
                    } else {
                        vec![path(p.map(&cut))]
                    }
                }
                Mark::Whole(i) => vec![path(p.map(&layer.outlines[*i]))],
                Mark::Paw(x, y, r) => {
                    // The badge's outline is only its extent: the paw itself
                    // is the body (so it survives `cursor_accent = "none"`)
                    // and the accent over it, sized to the *fitted* extent so
                    // grid fitting cannot push it past its clearance.
                    let (side, up, down) = PAW_REACH;
                    let (x0, y0) = p.px(x - side * r, y - up * r);
                    let (x1, y1) = p.px(x + side * r, y + down * r);
                    let pr = ((x1 - x0) / (2.0 * side)).min((y1 - y0) / (up + down));
                    let print = paw::prims((x0 + x1) / 2.0, y0 + up * pr, pr);
                    body = print.clone();
                    print
                }
                Mark::Spinner => {
                    let (cx, cy, _) = mocu::PROGRESS_DISK;
                    let (r0, r1) = mocu::PROGRESS_SPINNER;
                    let band = arc_band(
                        cx,
                        cy,
                        r0,
                        r1,
                        -std::f64::consts::FRAC_PI_2,
                        1.5 * std::f64::consts::PI,
                        size,
                    );
                    vec![path(p.map(&[band]))]
                }
            };
            // Every mark here is inlaid in its own layer's body (the paw
            // badge's body is the paw itself), so none wears a hairline.
            Layer {
                body,
                accent,
                accent_rim: false,
            }
        })
        .collect()
}
