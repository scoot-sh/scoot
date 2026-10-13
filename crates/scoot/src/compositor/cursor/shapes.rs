//! The procedurally-drawn cursor family, and which named shape each
//! [`CursorIcon`] maps onto.
//!
//! # Why these are drawn rather than loaded
//!
//! Same reason `cursor.rs`'s arrow always was: every pixel here is generated
//! from this module's own vector geometry, so nothing in the tree is a copy
//! of an xcursor theme's asset. niri's cursor assets are GPL and Adwaita's
//! are not MIT-clean (see `CLAUDE.md`'s license note), and this project has
//! no MIT-clean theme to load -- so a compositor that wants to answer
//! `wp_cursor_shape_device_v1.set_shape` with anything other than one
//! shape for every request has to draw the shapes itself. That is what this
//! module is: original geometry designed here, not a theme, and certainly
//! not a trace of the user's concept sheet (which is reference art only and
//! is never embedded, vendored or committed).
//!
//! # The family
//!
//! Pixel-art-leaning, built from the same three paints everywhere: the
//! configured body fill ([`Appearance::cursor_color`], near-black by
//! default), the auto-contrasting rim around it (white on a dark body, black
//! on a light one -- see `contrast_rim` in `super`), and the ginger accent
//! ([`Appearance::cursor_accent`], `None` draws no mark). The arrow carries
//! an orange hint at its tail; the badge shapes pair the arrow with a paw,
//! an hourglass or a "?" mark; the hand shapes are gloves with an orange
//! cuff (or knuckles); the resize/move/split shapes are white-headed arrows
//! around an orange centre line; the "not allowed" ring carries an orange
//! slash. See [`Shape::for_icon`] for how the sheet's names map onto the
//! [`CursorIcon`] set.
//!
//! # How a shape is built
//!
//! Each shape is described as two signed-distance fields over a `size x size`
//! grid -- body capsules/polygons/rings plus the accent mark's own
//! capsules/polys/disks -- and [`raster`](super::raster) evaluates both at
//! 4x4 subpixel taps per pixel, once per shape at startup or on config
//! reload. The rim sits *outside* every fill everywhere (a band
//! `outline_width` pixels wide); the accent mark gets its own hairline of
//! the same rim color, which puts a separating moat around on-body marks
//! (the tail hint, the cuff) and a halo around floating ones (the paw
//! badge). The arrow alone carries a soft offset shadow; the other shapes
//! do not (a blur on a 2px stroke reads as blur, not depth, and would break
//! their symmetry about the bitmap middle).
//!
//! Every dimension is a fraction of `size`, clamped into insets that leave
//! room for the rim (and the arrow's shadow) plus one pixel of
//! anti-aliasing bleed, so no fill pixel ever touches the bitmap edge at
//! any allowed size: there is always a rim pixel between the fill and
//! whatever is under the cursor, which is the only thing the rim is
//! for. Below 8px every shape degrades to an honest soft dot rather than a
//! legible drawing (hands and paws cannot read that small, and a half-drawn
//! one would be worse); nothing here pretends otherwise, and the tests
//! assert the insets hold at every size a config can ask for.
//!
//! All of this runs exactly once per shape, at startup, from
//! [`Cursor::new`](super::Cursor::new) -- never per frame and never per
//! `set_shape` request, which is an array index into buffers built here.
//!
//! The geometry lives in one submodule per family ([`arrow`], [`hand`],
//! [`paw`], [`text`], [`resize`], [`symbol`]) so no file sprawls; this file
//! owns the [`Shape`] enum, the icon mapping, hotspots and the single
//! [`generate`] entry point they all render through.

use smithay::input::pointer::CursorIcon;
use smithay::utils::{Logical, Point};

use super::raster::{self, Shadow};

pub mod arrow;
pub mod hand;
pub mod paw;
pub(crate) mod prim;
pub mod resize;
pub mod symbol;
pub mod text;

pub(crate) use prim::{
    Prim, arrow_pad_br, arrow_shadow, arrow_tip, chamfer, outline_width, span, stroke_width,
};

#[cfg(test)]
mod tests;

/// One drawable cursor shape.
///
/// Not a mirror of [`CursorIcon`] -- see [`Shape::for_icon`] for the mapping.
/// The set covers the concept sheet's family: the arrow, its three badges,
/// the text beams, the hands, the paw, the resize/move/split arrows and the
/// symbols.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// The pointer itself: slim dark arrow, white rim, orange tail hint.
    Arrow,
    /// Arrow plus a small orange paw badge at the lower right.
    PointerBadge,
    /// A thin I-beam with an orange stem mark.
    Text,
    /// The side I-beam, for vertical text, with an orange stem mark.
    VerticalText,
    /// A pointing glove with an orange cuff edge.
    Hand,
    /// A single orange paw print on the body color.
    Busy,
    /// Arrow plus a small hourglass badge.
    WaitBadge,
    /// A thin white cross with an orange centre dot.
    Crosshair,
    /// A thinner plain crosshair, for precise picking.
    Precision,
    /// A vertical double-headed arrow around an orange centre line.
    ResizeNs,
    /// A horizontal double-headed arrow around an orange centre line.
    ResizeEw,
    /// A double-headed arrow from bottom-left to top-right, dashed orange
    /// centre.
    ResizeNesw,
    /// A double-headed arrow from top-left to bottom-right, dashed orange
    /// centre.
    ResizeNwse,
    /// A four-headed arrow around an orange centre dot.
    Move,
    /// A ring with an orange slash through it.
    NotAllowed,
    /// Arrow plus a "?" badge.
    Help,
    /// An open hand, for grabbing.
    Grab,
    /// A closed fist with orange knuckles, for grabbing.
    Grabbing,
    /// Two triangles around a vertical orange divider.
    SplitV,
    /// Two triangles around a horizontal orange divider.
    SplitH,
    /// A magnifier with an orange plus.
    ZoomIn,
    /// A magnifier with an orange minus.
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
    /// not CSS names): the sheet's `default` is [`Shape::Arrow`]; its
    /// `pointer` (arrow plus paw badge) is [`Shape::PointerBadge`]; its
    /// `hand` (the glove) is [`Shape::Hand`] and therefore what CSS `pointer`
    /// draws; its `busy` paw is [`Shape::Busy`]; its `wait`
    /// (arrow plus hourglass) is [`Shape::WaitBadge`]; `help` is
    /// [`Shape::Help`]; `text`/`column-select` are [`Shape::Text`] and
    /// [`Shape::VerticalText`] (each carries the orange stem mark the sheet
    /// shows on its `text-select` cell); `crosshair`/`precision` are
    /// [`Shape::Crosshair`]/[`Shape::Precision`]; the four resize cells are
    /// the [`Shape::ResizeNs`]/[`Shape::ResizeEw`]/[`Shape::ResizeNesw`]/
    /// [`Shape::ResizeNwse`] arrows; `move` is [`Shape::Move`];
    /// `not-allowed` is [`Shape::NotAllowed`]; `grab`/`grabbing` are
    /// [`Shape::Grab`]/[`Shape::Grabbing`]; `split-vertical`/`split-horizontal`
    /// are [`Shape::SplitV`]/[`Shape::SplitH`], which is also what column and
    /// row resizing draw (a split control is what those operations act on).
    /// The magnifiers are new geometry in the same family (the sheet shows
    /// no zoom cell): a ring and handle in the body color with an orange
    /// plus or minus.
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
    /// actual location.
    ///
    /// The arrow family points with the arrow's tip, which sits `inset`
    /// pixels inside the bitmap's top-left corner (room for the outline and
    /// shadow -- see [`arrow_tip`]). The hand points with its fingertip.
    /// Every other shape here is symmetric about the pointer and means "the
    /// thing under the middle of me", so its hotspot is the middle pixel's
    /// corner. Below [`DOT_BELOW`] every shape is a centred dot (see
    /// [`generate`]), so every hotspot is the middle there too.
    ///
    /// `size` is the bitmap's edge length, already clamped by
    /// [`Cursor::new`](super::Cursor::new).
    pub fn hotspot(self, size: i32) -> Point<i32, Logical> {
        if size < DOT_BELOW {
            return (size / 2, size / 2).into();
        }
        match self {
            Shape::Arrow | Shape::PointerBadge | Shape::WaitBadge | Shape::Help => {
                let (x, y) = arrow_tip(size);
                (x, y).into()
            }
            Shape::Hand => hand::fingertip(size),
            _ => (size / 2, size / 2).into(),
        }
    }
}

/// Bitmaps smaller than this draw a centred soft dot, not a shape: hands,
/// paws and badges cannot read below it, and a half-drawn one would be worse
/// than an honest dot. [`Shape::hotspot`] agrees (middle) wherever this
/// applies, so the dot is always under the pointer.
const DOT_BELOW: i32 = 8;

/// Shared metrics for one bitmap size, computed once per [`generate`] call
/// and handed to whichever family module draws the shape.
pub(crate) struct Ctx {
    /// Bitmap edge length.
    pub size: i32,
    /// Rim width: 1px at the default size, growing slightly past it.
    pub ow: f64,
    /// Line-art stroke width: 2px at the default size so a stroke carries a
    /// solid core inside its own rim on both sides.
    pub w: f64,
    /// Accent hairline width: half the rim, so the mark reads as inlaid
    /// rather than bordered.
    pub aow: f64,
    /// Bitmap middle on the half-pixel grid (the middle of a pixel column),
    /// so strokes cover whole pixels instead of straddling two greyly.
    pub c: f64,
    /// Corner rounding for line-art polygons: subpixel at 16px, a real
    /// radius once there are pixels to spend on it.
    pub round: f64,
}

impl Ctx {
    fn new(size: i32) -> Self {
        let ow = outline_width(size);
        Self {
            size,
            ow,
            w: stroke_width(size),
            aow: (ow * 0.5).max(0.5),
            c: f64::from(size / 2) + 0.5,
            round: (f64::from(size) / 20.0).max(0.8),
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
/// family: the same rim width, the same accent hairline, the same
/// supersampling, the same premultiplication.
pub fn generate(
    shape: Shape,
    size: i32,
    fill: [u8; 4],
    outline: [u8; 4],
    accent: Option<[u8; 4]>,
) -> Vec<u8> {
    let ctx = Ctx::new(size);
    let alpha = f64::from(fill[3]) / 255.0;
    // Below `DOT_BELOW` every shape is one honest dot at the middle (which
    // is also the hotspot there): hands and badges cannot read that small.
    if size < DOT_BELOW {
        let c = f64::from(size) / 2.0;
        let dot = Prim::Disk {
            cx: c,
            cy: c,
            r: ctx.w / 2.0,
        };
        let mut bounds = dot.bounds();
        let pad = ctx.ow + 1.0;
        bounds[0] -= pad;
        bounds[1] -= pad;
        bounds[2] += pad;
        bounds[3] += pad;
        let field = move |px: f64, py: f64| dot.distance(px, py);
        let body = raster::Coat {
            fill,
            outline,
            outline_width: ctx.ow,
        };
        return raster::render(size, &field, body, None, None, Some(bounds));
    }
    let (prims, accent_prims, shadow) = field(shape, &ctx, alpha);
    // The rasterizer's skip box: the union of both layers' primitives,
    // expanded past the rims, the shadow and one pixel of bleed. Taps
    // outside it are transparent by construction, so skipping the fields
    // there changes nothing and keeps large bitmaps cheap.
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
    if let Some(sh) = shadow {
        // The shadow samples the field up-left of the tap, so it paints
        // down-right of the fill.
        bounds[2] += sh.dx + sh.feather;
        bounds[3] += sh.dy + sh.feather;
    }
    let body = move |px: f64, py: f64| {
        prims
            .iter()
            .map(|p| p.distance(px, py))
            .fold(f64::INFINITY, raster::union)
    };
    let body_coat = raster::Coat {
        fill,
        outline,
        outline_width: ctx.ow,
    };
    // No accent paint or no mark: the rasterizer skips the second field.
    if accent.is_none() || accent_prims.is_empty() {
        return raster::render(size, &body, body_coat, None, shadow, Some(bounds));
    }
    let mark = move |px: f64, py: f64| {
        accent_prims
            .iter()
            .map(|p| p.distance(px, py))
            .fold(f64::INFINITY, raster::union)
    };
    let coat = raster::Accent {
        field: &mark,
        coat: raster::Coat {
            fill: accent.expect("checked above"),
            outline,
            outline_width: ctx.aow,
        },
    };
    raster::render(size, &body, body_coat, Some(coat), shadow, Some(bounds))
}

/// Builds the shape's body primitives, its accent-mark primitives and its
/// shadow (which is `Some` only for the arrow family on bitmaps large enough
/// to hold one).
fn field(shape: Shape, ctx: &Ctx, alpha: f64) -> (Vec<Prim>, Vec<Prim>, Option<Shadow>) {
    match shape {
        Shape::Arrow => arrow::arrow(ctx, alpha),
        Shape::PointerBadge => arrow::pointer_badge(ctx, alpha),
        Shape::WaitBadge => arrow::wait_badge(ctx, alpha),
        Shape::Help => arrow::help(ctx, alpha),
        Shape::Text => text::text(ctx),
        Shape::VerticalText => text::vertical_text(ctx),
        Shape::Hand => hand::hand(ctx),
        Shape::Grab => hand::grab(ctx),
        Shape::Grabbing => hand::grabbing(ctx),
        Shape::Busy => paw::busy(ctx),
        Shape::Crosshair => symbol::crosshair(ctx),
        Shape::Precision => symbol::precision(ctx),
        Shape::NotAllowed => symbol::not_allowed(ctx),
        Shape::ZoomIn => symbol::zoom_in(ctx),
        Shape::ZoomOut => symbol::zoom_out(ctx),
        Shape::ResizeNs => resize::resize_ns(ctx),
        Shape::ResizeEw => resize::resize_ew(ctx),
        Shape::ResizeNesw => resize::resize_nesw(ctx),
        Shape::ResizeNwse => resize::resize_nwse(ctx),
        Shape::Move => resize::move_shape(ctx),
        Shape::SplitV => resize::split_v(ctx),
        Shape::SplitH => resize::split_h(ctx),
    }
}
