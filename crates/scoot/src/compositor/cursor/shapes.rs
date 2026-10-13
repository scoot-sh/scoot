//! The cursor shapes scoot draws where no cursor theme is installed: Mocu,
//! and which Mocu cursor each [`CursorIcon`] maps onto.
//!
//! # Mocu
//!
//! Every shape is Mocu, Severin Meyer's "modest xcursor theme", released
//! under CC0 1.0 (public domain, so free to use in this MIT project with no
//! conditions). Its SVG sources are vendored unchanged with their license
//! under `shapes/mocu/` and credited in `NOTICE`; [`mocu`] reads them,
//! [`svgpath`] flattens their outlines, [`place`] puts each in the bitmap
//! and [`raster`](super::raster) fills it -- the same layers Mocu's own
//! build renders: a faint drop shadow, each outline stroked and filled, and
//! the busy spinner. Nothing is redrawn, recolored beyond Mocu's own two
//! paints, or added.
//!
//! These are drawn rather than loaded because a machine with no cursor theme
//! installed (a webtop, a minimal container) is the everyday case this
//! fallback exists for, and `wp_cursor_shape_device_v1.set_shape` deserves
//! the right shape for every request there.
//!
//! # Paints
//!
//! Mocu's fill is [`Appearance::cursor_color`] (Mocu's white, `#fcfcfc`, by
//! default) and its stroke the paired rim `contrast_rim` derives (Mocu's
//! `#222222` around a light fill, its `#f9f9f9` around a dark one -- Mocu's
//! black variant). The shadow is Mocu's: black at 10% opacity, one unit
//! down and right.
//!
//! [`Appearance::cursor_color`]: crate::compositor::decorations::Appearance::cursor_color
//!
//! All of this runs once per shape per output scale, at startup or on
//! config reload -- never per frame and never per `set_shape` request,
//! which is an array index into buffers built here. Below 8px every shape is
//! an honest dot under the pointer rather than a half-legible drawing.

use smithay::input::pointer::CursorIcon;
use smithay::utils::{Logical, Point};

use super::raster;
use svgpath::Subpath;

pub(crate) mod mocu;
pub(crate) mod place;
pub(crate) mod svgpath;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_mocu;

/// One drawable cursor: one Mocu cursor, named after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Default,
    ContextMenu,
    Help,
    Pointer,
    Progress,
    Wait,
    Cell,
    Crosshair,
    Text,
    VerticalText,
    Alias,
    Copy,
    NoDrop,
    NotAllowed,
    Grab,
    Grabbing,
    EResize,
    NResize,
    NeResize,
    NwResize,
    SResize,
    SeResize,
    SwResize,
    WResize,
    EwResize,
    NsResize,
    NeswResize,
    NwseResize,
    ColResize,
    RowResize,
    AllScroll,
    ZoomIn,
    ZoomOut,
    DndAsk,
}

impl Shape {
    /// Every shape, in the order [`Shape::index`] numbers them -- the same
    /// order as [`mocu::FILES`], which is how a shape finds its file.
    pub const ALL: [Shape; Self::COUNT] = [
        Shape::Default,
        Shape::ContextMenu,
        Shape::Help,
        Shape::Pointer,
        Shape::Progress,
        Shape::Wait,
        Shape::Cell,
        Shape::Crosshair,
        Shape::Text,
        Shape::VerticalText,
        Shape::Alias,
        Shape::Copy,
        Shape::NoDrop,
        Shape::NotAllowed,
        Shape::Grab,
        Shape::Grabbing,
        Shape::EResize,
        Shape::NResize,
        Shape::NeResize,
        Shape::NwResize,
        Shape::SResize,
        Shape::SeResize,
        Shape::SwResize,
        Shape::WResize,
        Shape::EwResize,
        Shape::NsResize,
        Shape::NeswResize,
        Shape::NwseResize,
        Shape::ColResize,
        Shape::RowResize,
        Shape::AllScroll,
        Shape::ZoomIn,
        Shape::ZoomOut,
        Shape::DndAsk,
    ];

    /// How many shapes there are, i.e. how many bitmaps
    /// [`Cursor`](super::Cursor) holds per output scale.
    pub const COUNT: usize = 34;

    /// This shape's slot in [`Shape::ALL`] and [`mocu::FILES`].
    pub fn index(self) -> usize {
        self as usize
    }

    /// The Mocu cursor name this shape draws (its file under `mocu/`).
    #[cfg(test)]
    pub fn name(self) -> &'static str {
        mocu::FILES[self.index()].0
    }

    /// The shape drawn for a client's requested [`CursorIcon`]: Mocu's
    /// cursor of the same name. Mocu has a file for every CSS cursor name
    /// but two, which take Mocu's own alias for them: `move` is Mocu's
    /// `grabbing` (Mocu's `aliases.txt`: `move grabbing`), and `all-resize`,
    /// which Mocu does not name, is its four-way `all-scroll`.
    ///
    /// Total by construction -- a `_` arm, not an enumeration of the rest:
    /// [`CursorIcon`] is a non-exhaustive upstream enum, so a version bump
    /// that adds an icon must keep compiling and must show *something*
    /// rather than nothing. [`Shape::Default`] is that something.
    pub fn for_icon(icon: CursorIcon) -> Shape {
        match icon {
            CursorIcon::ContextMenu => Shape::ContextMenu,
            CursorIcon::Help => Shape::Help,
            CursorIcon::Pointer => Shape::Pointer,
            CursorIcon::Progress => Shape::Progress,
            CursorIcon::Wait => Shape::Wait,
            CursorIcon::Cell => Shape::Cell,
            CursorIcon::Crosshair => Shape::Crosshair,
            CursorIcon::Text => Shape::Text,
            CursorIcon::VerticalText => Shape::VerticalText,
            CursorIcon::Alias => Shape::Alias,
            CursorIcon::Copy => Shape::Copy,
            CursorIcon::Move => Shape::Grabbing,
            CursorIcon::NoDrop => Shape::NoDrop,
            CursorIcon::NotAllowed => Shape::NotAllowed,
            CursorIcon::Grab => Shape::Grab,
            CursorIcon::Grabbing => Shape::Grabbing,
            CursorIcon::EResize => Shape::EResize,
            CursorIcon::NResize => Shape::NResize,
            CursorIcon::NeResize => Shape::NeResize,
            CursorIcon::NwResize => Shape::NwResize,
            CursorIcon::SResize => Shape::SResize,
            CursorIcon::SeResize => Shape::SeResize,
            CursorIcon::SwResize => Shape::SwResize,
            CursorIcon::WResize => Shape::WResize,
            CursorIcon::EwResize => Shape::EwResize,
            CursorIcon::NsResize => Shape::NsResize,
            CursorIcon::NeswResize => Shape::NeswResize,
            CursorIcon::NwseResize => Shape::NwseResize,
            CursorIcon::ColResize => Shape::ColResize,
            CursorIcon::RowResize => Shape::RowResize,
            CursorIcon::AllScroll | CursorIcon::AllResize => Shape::AllScroll,
            CursorIcon::ZoomIn => Shape::ZoomIn,
            CursorIcon::ZoomOut => Shape::ZoomOut,
            CursorIcon::DndAsk => Shape::DndAsk,
            _ => Shape::Default,
        }
    }

    /// The point within this shape's bitmap that lines up with the pointer's
    /// actual location: Mocu's own hotspot (its `<circle id="hot">`), carried
    /// through the same placement as the outline -- the arrow's tip, the
    /// hand's fingertip, the middle of the symmetric shapes. Below
    /// [`DOT_BELOW`] every shape is a centred dot, so the hotspot is the
    /// middle there.
    ///
    /// Places the outline to answer, so it is a cold-path call:
    /// [`Cursor`](super::Cursor) computes it once per bitmap and keeps it.
    pub fn hotspot(self, size: i32) -> Point<i32, Logical> {
        if size < DOT_BELOW {
            return (size / 2, size / 2).into();
        }
        place::place(&self.art(size), size).hotspot()
    }

    /// This shape's Mocu art, curves flattened for a `size` bitmap.
    pub(crate) fn art(self, size: i32) -> mocu::Art {
        mocu::read(mocu::FILES[self.index()].1, tolerance(size))
    }
}

/// Bitmaps smaller than this draw a centred soft dot, not a shape.
/// [`Shape::hotspot`] agrees (middle) wherever this applies.
const DOT_BELOW: i32 = 8;

/// Curves flatten to within a tenth of a pixel, in grid units.
fn tolerance(size: i32) -> f64 {
    0.1 * 24.0 / f64::from(size.max(1))
}

/// An outline's bounding box grown by `pad` pixels: the rasterizer's skip
/// box (taps outside it cannot be covered).
fn bounds(subs: &[Subpath], pad: f64) -> Option<[f64; 4]> {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for (x, y) in subs.iter().flatten() {
        b = [b[0].min(*x), b[1].min(*y), b[2].max(*x), b[3].max(*y)];
    }
    Some([b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad])
}

/// Draws `shape` into a fresh `size * size * 4` byte `Argb8888` buffer.
///
/// `fill` is Mocu's fill paint and `stroke` its stroke paint, both single
/// premultiplied little-endian `Argb8888` pixels (`[B, G, R, A]`), the same
/// layout the render path scans out. Mocu's layers stack back to front: the
/// shadow (black at 10% of the fill's alpha), then each outline's stroke and
/// fill, then the spinner (the stroke paint seen through Mocu's mask).
pub fn generate(shape: Shape, size: i32, fill: [u8; 4], stroke: [u8; 4]) -> Vec<u8> {
    // Below `DOT_BELOW` every shape is one honest dot at the middle (which
    // is also the hotspot there).
    if size < DOT_BELOW {
        let c = f64::from(size) / 2.0;
        let dot = vec![mocu::circle(c, c, 1.0, 0.05)];
        let field = |x: f64, y: f64| raster::sd_path(x, y, &dot);
        let coat = raster::Coat {
            fill,
            outline: stroke,
            outline_width: 1.0,
        };
        return raster::render(size, &field, coat, None, bounds(&dot, 2.0));
    }
    let art = shape.art(size);
    let p = place::place(&art, size);
    let layers: Vec<(Vec<Subpath>, f64)> = art
        .layers
        .iter()
        .map(|l| (p.map(&l.subs), place::rim(l.stroke, size)))
        .collect();
    // The shadow: every layer grown by its stroke, one unit down and right,
    // as one union -- Mocu paints it as one group at 10% -- and only where
    // no stroke or fill covers: invisible under opaque paint anyway, and
    // under a translucent `cursor_color` it would darken the fill rather
    // than let the background show through it.
    let alpha = f64::from(fill[3]) / 255.0;
    let off = place::shadow_offset(size);
    let all: Vec<Subpath> = layers.iter().flat_map(|(s, _)| s.clone()).collect();
    let reach = layers.iter().map(|(_, r)| *r).fold(0.0, f64::max);
    let outer = |x: f64, y: f64| {
        layers
            .iter()
            .map(|(subs, rim)| raster::sd_path(x, y, subs) - rim)
            .fold(f64::INFINITY, f64::min)
    };
    let shadow_field = |x: f64, y: f64| outer(x - off, y - off).max(-outer(x, y));
    let shadow = raster::Coat {
        fill: [0, 0, 0, (0.1 * alpha * 255.0).round() as u8],
        outline: [0; 4],
        outline_width: 0.0,
    };
    let shadow_box = bounds(&all, reach + 1.0).map(|b| [b[0], b[1], b[2] + off, b[3] + off]);
    let mut out = raster::render(size, &shadow_field, shadow, None, shadow_box);
    for (subs, rim) in &layers {
        let field = |x: f64, y: f64| raster::sd_path(x, y, subs);
        let coat = raster::Coat {
            fill,
            outline: stroke,
            outline_width: *rim,
        };
        let pixels = raster::render(size, &field, coat, None, bounds(subs, rim + 1.0));
        raster::over(&mut out, &pixels);
    }
    if let Some(spinner) = &art.spinner {
        let disk = p.map(std::slice::from_ref(&spinner.disk));
        let mask: Vec<(Vec<Subpath>, f64)> = spinner
            .mask
            .iter()
            .map(|(subs, lum)| (p.map(subs), *lum))
            .collect();
        let field = |x: f64, y: f64| raster::sd_path(x, y, &disk);
        // The last mask shape covering a point decides its opacity.
        let opacity = |x: f64, y: f64| {
            mask.iter()
                .rev()
                .find(|(subs, _)| raster::inside(x, y, subs))
                .map_or(0.0, |(_, lum)| *lum)
        };
        let coat = raster::Coat {
            fill: stroke,
            outline: [0; 4],
            outline_width: 0.0,
        };
        let pixels = raster::render(size, &field, coat, Some(&opacity), bounds(&disk, 1.0));
        raster::over(&mut out, &pixels);
    }
    out
}
