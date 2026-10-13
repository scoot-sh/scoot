//! The cursor family scoot draws itself, and which named shape each
//! [`CursorIcon`] maps onto.
//!
//! # Where the shapes come from
//!
//! Most outlines are Mocu's -- Severin Meyer's "modest xcursor theme",
//! released under CC0 1.0 (public domain, so free to use in this MIT project
//! with no conditions; vendored with its license under `shapes/mocu/` and
//! credited in `NOTICE`). [`mocu`] holds the SVG path data copied verbatim
//! from those files; [`svgpath`] flattens it and [`family`] places it in the
//! bitmap and cuts the accent marks out of it. Mocu has no paw, so the busy
//! paw and the pointer's paw badge are scoot's own geometry ([`paw`]). The
//! user's concept sheet is reference for intent only: nothing is traced
//! from it, and it is never embedded, vendored or committed.
//!
//! These are drawn rather than loaded because a machine with no cursor theme
//! installed (a webtop, a minimal container) is the everyday case this
//! fallback exists for, and `wp_cursor_shape_device_v1.set_shape` deserves
//! more than one arrow for every request there.
//!
//! # The family
//!
//! Every shape is painted the way Mocu paints its own: the outline filled
//! with the body color ([`Appearance::cursor_color`], near-black by
//! default), ringed by the auto-contrasting rim (white on a dark body, black
//! on a light one -- see `contrast_rim` in `super`). On top sits the ginger
//! accent ([`Appearance::cursor_accent`], `None` draws no mark) where it
//! reads at 16px: the arrow's tail tip, the resize and split shafts, the
//! move centre, the not-allowed slash, the hand's cuff, the magnifier's sign
//! and the wait spinner. See [`Shape::for_icon`] for how CSS names map onto
//! the shapes.
//!
//! # How a shape is built
//!
//! Each shape is two signed-distance fields over a `size x size` grid -- the
//! body outlines plus the accent mark -- and [`raster`](super::raster)
//! evaluates both at 4x4 subpixel taps per pixel, once per shape per output
//! scale at startup or on config reload (never per frame, never per
//! `set_shape` request, which is an array index into buffers built here).
//! The rim sits *outside* every fill (a band `outline_width` pixels wide);
//! a mark that floats off the body (the paw badge) gets its own rim-colored
//! hairline, a mark inlaid in the body does not.
//!
//! Every shape keeps its rim clear of the bitmap edge at every size a config
//! can ask for: there is always a transparent pixel between the rim and the
//! edge, so the rim is never clipped, and the tests assert it. Below 8px
//! every shape degrades to an honest soft dot under the pointer rather than
//! a half-legible drawing.

use smithay::input::pointer::CursorIcon;
use smithay::utils::{Logical, Point};

use super::raster;

pub mod family;
pub(crate) mod mocu;
pub mod paw;
pub(crate) mod prim;
pub(crate) mod svgpath;

pub(crate) use prim::{Prim, outline_width};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_mocu;

/// One drawable cursor shape.
///
/// Not a mirror of [`CursorIcon`] -- see [`Shape::for_icon`] for the mapping.
/// The set covers the concept sheet's family: the arrow, its three badges,
/// the text beams, the hands, the paw, the resize/move/split arrows and the
/// symbols.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// The pointer itself: Mocu's arrow, ginger tail tip.
    Arrow,
    /// The arrow plus a ginger paw badge at the lower right (progress).
    PointerBadge,
    /// Mocu's I-beam.
    Text,
    /// Mocu's sideways I-beam, for vertical text.
    VerticalText,
    /// Mocu's pointing hand, ginger cuff.
    Hand,
    /// A single ginger paw print (scoot's own geometry).
    Busy,
    /// The arrow plus Mocu's progress badge, its spinner frozen in ginger.
    WaitBadge,
    /// Mocu's hairline crosshair, ginger centre.
    Crosshair,
    /// Mocu's bold `cell` plus, for picking cells.
    Precision,
    /// Mocu's vertical double arrow, ginger shaft.
    ResizeNs,
    /// Mocu's horizontal double arrow, ginger shaft.
    ResizeEw,
    /// Mocu's bottom-left to top-right double arrow, ginger shaft.
    ResizeNesw,
    /// Mocu's top-left to bottom-right double arrow, ginger shaft.
    ResizeNwse,
    /// Mocu's four-way arrows around a ginger centre disk.
    Move,
    /// Mocu's ring, ginger slash.
    NotAllowed,
    /// Mocu's arrow plus question mark.
    Help,
    /// Mocu's open hand.
    Grab,
    /// Mocu's closed hand.
    Grabbing,
    /// Mocu's column resize: arrows either side of a ginger bar.
    SplitV,
    /// Mocu's row resize: arrows above and below a ginger bar.
    SplitH,
    /// Mocu's magnifier, ginger plus.
    ZoomIn,
    /// Mocu's magnifier, ginger minus.
    ZoomOut,
}

impl Shape {
    /// Every shape, in the order [`Shape::index`] numbers them.
    ///
    /// The one place that order is written down -- [`Cursor`](super::Cursor)
    /// builds its buffers by walking this, and reads them back by
    /// [`Shape::index`], so the two cannot drift apart.
    pub const ALL: [Shape; Self::COUNT] = [
        Shape::Arrow,
        Shape::PointerBadge,
        Shape::Text,
        Shape::VerticalText,
        Shape::Hand,
        Shape::Busy,
        Shape::WaitBadge,
        Shape::Crosshair,
        Shape::Precision,
        Shape::ResizeNs,
        Shape::ResizeEw,
        Shape::ResizeNesw,
        Shape::ResizeNwse,
        Shape::Move,
        Shape::NotAllowed,
        Shape::Help,
        Shape::Grab,
        Shape::Grabbing,
        Shape::SplitV,
        Shape::SplitH,
        Shape::ZoomIn,
        Shape::ZoomOut,
    ];

    /// How many shapes there are, i.e. how many bitmaps
    /// [`Cursor`](super::Cursor) holds.
    pub const COUNT: usize = 22;

    /// This shape's slot in [`Shape::ALL`].
    pub fn index(self) -> usize {
        match self {
            Shape::Arrow => 0,
            Shape::PointerBadge => 1,
            Shape::Text => 2,
            Shape::VerticalText => 3,
            Shape::Hand => 4,
            Shape::Busy => 5,
            Shape::WaitBadge => 6,
            Shape::Crosshair => 7,
            Shape::Precision => 8,
            Shape::ResizeNs => 9,
            Shape::ResizeEw => 10,
            Shape::ResizeNesw => 11,
            Shape::ResizeNwse => 12,
            Shape::Move => 13,
            Shape::NotAllowed => 14,
            Shape::Help => 15,
            Shape::Grab => 16,
            Shape::Grabbing => 17,
            Shape::SplitV => 18,
            Shape::SplitH => 19,
            Shape::ZoomIn => 20,
            Shape::ZoomOut => 21,
        }
    }

    /// The shape drawn for a client's requested [`CursorIcon`].
    ///
    /// Total by construction -- a `_` arm, not an enumeration of the rest:
    /// [`CursorIcon`] is a non-exhaustive upstream enum, so a version bump
    /// that adds an icon must keep compiling and must show *something*
    /// rather than nothing. [`Shape::Arrow`] is that something.
    ///
    /// The mapping from the concept sheet's names (which are the user's own,
    /// not CSS names): `default` is [`Shape::Arrow`]; `pointer` (arrow plus
    /// paw badge) is [`Shape::PointerBadge`], what CSS `progress` draws;
    /// `hand` is [`Shape::Hand`], what CSS `pointer` draws; `busy` is
    /// [`Shape::Busy`]; `wait` is [`Shape::WaitBadge`] (Mocu's progress badge
    /// rather than the sheet's hourglass: Mocu's art wins where both exist);
    /// `help` is [`Shape::Help`]; `text`/`column-select` are [`Shape::Text`]
    /// and [`Shape::VerticalText`] (CSS `vertical-text` is the sideways
    /// beam, so that is how it is drawn); `crosshair`/`precision` are
    /// [`Shape::Crosshair`]/[`Shape::Precision`] (Mocu's `cell`); the four
    /// resize cells are the resize arrows; `move` is [`Shape::Move`];
    /// `not-allowed` is [`Shape::NotAllowed`]; `grab`/`grabbing` are
    /// [`Shape::Grab`]/[`Shape::Grabbing`]; `split-vertical`/
    /// `split-horizontal` are [`Shape::SplitV`]/[`Shape::SplitH`] (Mocu's
    /// `col-resize`/`row-resize`). The magnifiers have no sheet cell; they
    /// are Mocu's `zoom-in`/`zoom-out`.
    pub fn for_icon(icon: CursorIcon) -> Shape {
        match icon {
            CursorIcon::Text => Shape::Text,
            CursorIcon::VerticalText => Shape::VerticalText,
            CursorIcon::Crosshair => Shape::Crosshair,
            CursorIcon::Cell => Shape::Precision,
            CursorIcon::NResize | CursorIcon::SResize | CursorIcon::NsResize => Shape::ResizeNs,
            CursorIcon::EResize | CursorIcon::WResize | CursorIcon::EwResize => Shape::ResizeEw,
            CursorIcon::NeResize | CursorIcon::SwResize | CursorIcon::NeswResize => {
                Shape::ResizeNesw
            }
            CursorIcon::NwResize | CursorIcon::SeResize | CursorIcon::NwseResize => {
                Shape::ResizeNwse
            }
            CursorIcon::ColResize => Shape::SplitV,
            CursorIcon::RowResize => Shape::SplitH,
            CursorIcon::Move | CursorIcon::AllScroll | CursorIcon::AllResize => Shape::Move,
            CursorIcon::Grab => Shape::Grab,
            CursorIcon::Grabbing => Shape::Grabbing,
            CursorIcon::Pointer => Shape::Hand,
            CursorIcon::Progress | CursorIcon::Alias | CursorIcon::Copy => Shape::PointerBadge,
            CursorIcon::Wait => Shape::WaitBadge,
            CursorIcon::DndAsk => Shape::Busy,
            CursorIcon::Help => Shape::Help,
            CursorIcon::ZoomIn => Shape::ZoomIn,
            CursorIcon::ZoomOut => Shape::ZoomOut,
            CursorIcon::NotAllowed | CursorIcon::NoDrop => Shape::NotAllowed,
            _ => Shape::Arrow,
        }
    }

    /// The point within this shape's bitmap that lines up with the pointer's
    /// actual location: Mocu's own hotspot for the shape, carried through the
    /// same placement as its outline (see [`family`]) -- the arrow's tip, the
    /// hand's fingertip, the middle of the symmetric shapes. Below
    /// [`DOT_BELOW`] every shape is a centred dot (see [`generate`]), so every
    /// hotspot is the middle there.
    ///
    /// Places the shape's outline to answer, so it is a cold-path call:
    /// [`Cursor`](super::Cursor) computes it once per bitmap and keeps it.
    /// `size` is the bitmap's edge length, already clamped by
    /// [`Cursor::new`](super::Cursor::new).
    pub fn hotspot(self, size: i32) -> Point<i32, Logical> {
        if size < DOT_BELOW {
            return (size / 2, size / 2).into();
        }
        match self.art() {
            Some(art) => family::placement(art, size).hotspot(),
            None => (size / 2, size / 2).into(),
        }
    }

    /// The Mocu art this shape is drawn from, or `None` for scoot's own
    /// (the busy paw).
    fn art(self) -> Option<family::Art> {
        use family::Art;
        Some(match self {
            Shape::Arrow => Art::Arrow,
            Shape::PointerBadge => Art::PointerBadge,
            Shape::Text => Art::Text,
            Shape::VerticalText => Art::VerticalText,
            Shape::Hand => Art::Hand,
            Shape::Busy => return None,
            Shape::WaitBadge => Art::WaitBadge,
            Shape::Crosshair => Art::Crosshair,
            Shape::Precision => Art::Cell,
            Shape::ResizeNs => Art::ResizeNs,
            Shape::ResizeEw => Art::ResizeEw,
            Shape::ResizeNesw => Art::ResizeNesw,
            Shape::ResizeNwse => Art::ResizeNwse,
            Shape::Move => Art::Move,
            Shape::NotAllowed => Art::NotAllowed,
            Shape::Help => Art::Help,
            Shape::Grab => Art::Grab,
            Shape::Grabbing => Art::Grabbing,
            Shape::SplitV => Art::SplitV,
            Shape::SplitH => Art::SplitH,
            Shape::ZoomIn => Art::ZoomIn,
            Shape::ZoomOut => Art::ZoomOut,
        })
    }
}

/// Bitmaps smaller than this draw a centred soft dot, not a shape: hands,
/// paws and badges cannot read below it, and a half-drawn one would be worse
/// than an honest dot. [`Shape::hotspot`] agrees (middle) wherever this
/// applies, so the dot is always under the pointer.
const DOT_BELOW: i32 = 8;

/// Shared metrics for one bitmap size, computed once per [`generate`] call.
pub(crate) struct Ctx {
    /// Bitmap edge length.
    pub size: i32,
    /// One 24-grid unit in pixels (`size / 24`).
    pub u: f64,
    /// Rim width: 1px at the default size, growing past it.
    pub ow: f64,
    /// Hairline width around a floating accent mark: half the rim.
    pub aow: f64,
    /// Bitmap middle on the half-pixel grid (the middle of a pixel column).
    pub c: f64,
}

impl Ctx {
    fn new(size: i32) -> Self {
        let ow = outline_width(size);
        Self {
            size,
            u: f64::from(size) / 24.0,
            ow,
            aow: (ow * 0.5).max(0.5),
            c: f64::from(size / 2) + 0.5,
        }
    }
}

/// Draws `shape` into a fresh `size * size * 4` byte `Argb8888` buffer.
///
/// `fill` is the body color and `outline` its auto-contrasting rim, both
/// single premultiplied little-endian `Argb8888` pixels (`[B, G, R, A]`), the
/// same layout `cursor.rs` has always used and the same one the render path
/// scans out. `accent` is the family mark's paint (`None` draws no mark).
/// Every shape renders through this function, so the whole set is one
/// family: the same rim width, the same supersampling, the same
/// premultiplication.
pub fn generate(
    shape: Shape,
    size: i32,
    fill: [u8; 4],
    outline: [u8; 4],
    accent: Option<[u8; 4]>,
) -> Vec<u8> {
    let ctx = Ctx::new(size);
    let body_coat = raster::Coat {
        fill,
        outline,
        outline_width: ctx.ow,
    };
    // Below `DOT_BELOW` every shape is one honest dot at the middle (which
    // is also the hotspot there): hands and badges cannot read that small.
    if size < DOT_BELOW {
        let c = f64::from(size) / 2.0;
        let dot = Prim::Disk {
            cx: c,
            cy: c,
            r: 1.0,
        };
        let mut bounds = dot.bounds();
        let pad = ctx.ow + 1.0;
        bounds[0] -= pad;
        bounds[1] -= pad;
        bounds[2] += pad;
        bounds[3] += pad;
        let field = move |px: f64, py: f64| dot.distance(px, py);
        return raster::render(size, &field, body_coat, None, Some(bounds));
    }
    let layers = match shape.art() {
        Some(art) => family::draw(art, &ctx),
        None => vec![paw::busy(&ctx)],
    };
    // One render per coat, each later coat composited over the earlier ones
    // (a badge's own rim over the arrow it sits on). Nearly every shape is
    // a single coat, which is a single render.
    let mut out: Option<Vec<u8>> = None;
    for layer in layers {
        let pixels = render_layer(size, &ctx, layer, body_coat, outline, accent);
        match out.as_mut() {
            None => out = Some(pixels),
            Some(dst) => raster::over(dst, &pixels),
        }
    }
    out.unwrap_or_else(|| vec![0; (size * size * 4) as usize])
}

/// Rasterises one [`family::Layer`]: its body in `body_coat`, its accent
/// mark (if any, and if `accent` paint is configured) over it.
fn render_layer(
    size: i32,
    ctx: &Ctx,
    layer: family::Layer,
    body_coat: raster::Coat,
    outline: [u8; 4],
    accent: Option<[u8; 4]>,
) -> Vec<u8> {
    let family::Layer {
        body: prims,
        accent: accent_prims,
        accent_rim,
    } = layer;
    // The rasterizer's skip box: the union of both fields' primitives,
    // expanded past the rims and one pixel of bleed. Taps outside it are
    // transparent by construction, so skipping the fields there changes
    // nothing and keeps large bitmaps cheap.
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for prim in prims.iter().chain(accent_prims.iter()) {
        let b = prim.bounds();
        bounds[0] = bounds[0].min(b[0]);
        bounds[1] = bounds[1].min(b[1]);
        bounds[2] = bounds[2].max(b[2]);
        bounds[3] = bounds[3].max(b[3]);
    }
    let pad = ctx.ow.max(ctx.aow) + 1.0;
    bounds[0] -= pad;
    bounds[1] -= pad;
    bounds[2] += pad;
    bounds[3] += pad;
    let body = move |px: f64, py: f64| {
        prims
            .iter()
            .map(|p| p.distance(px, py))
            .fold(f64::INFINITY, raster::union)
    };
    // No accent paint or no mark: the rasterizer skips the second field.
    let Some(accent) = accent.filter(|_| !accent_prims.is_empty()) else {
        return raster::render(size, &body, body_coat, None, Some(bounds));
    };
    let mark = move |px: f64, py: f64| {
        accent_prims
            .iter()
            .map(|p| p.distance(px, py))
            .fold(f64::INFINITY, raster::union)
    };
    let coat = raster::Accent {
        field: &mark,
        coat: raster::Coat {
            fill: accent,
            outline,
            outline_width: if accent_rim { ctx.aow } else { 0.0 },
        },
    };
    raster::render(size, &body, body_coat, Some(coat), Some(bounds))
}
