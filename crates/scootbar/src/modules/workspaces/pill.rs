//! The active workspace's pill: its shape, and where it sits. Pure, so the
//! draw and the hit test share one answer ([`Pill::extent`]) and both are
//! unit tests.
//!
//! Four shapes, by `[workspaces] pill-shape`:
//!
//! - **`rect`**: the item's span padded by half the module padding,
//!   square unless `pill-radius` rounds it, the bar's full height
//!   unless `pill-inset` lifts it.
//! - **`pill`**: the same extent with the radius as large as fits, so the
//!   ends are half circles.
//! - **`circle`**: a pill that is at least as wide as it is tall, centered
//!   on the number, so a single digit sits in a disc. A number too wide
//!   for the disc (two digits, on a small bar) **widens into a pill** the
//!   width of its text, never a clipped disc; and the growth stops short of
//!   the neighbouring numbers, so a crowded row gets a narrower pill
//!   rather than one covering a neighbour.
//! - **`square`** (the default): a rect that is at least as wide as it is tall, centered
//!   on the number, so a single digit sits in a square tile at any
//!   padding. Like a circle but square: a wider number widens it into a
//!   wider tile, never clipped, stopping short of the neighbours. The
//!   corners are `pill-radius` (a small radius reads best); with `disc`
//!   the module's span grows to the tile, as for a circle.
//!
//! Vertically the pill is the bar's height less `pill-inset` at the top
//! and bottom, the inset cut back so the pill stays as tall as the text's
//! line: the number is drawn in the background color over the fill, and a
//! pill shorter than it would clip the number away.

use crate::density::Scale;
use crate::render::device;

#[cfg(test)]
mod tests;

/// The pill's shape.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Shape {
    #[default]
    Rect,
    Pill,
    Circle,
    Square,
}

impl Shape {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "rect" => Some(Self::Rect),
            "pill" => Some(Self::Pill),
            "circle" => Some(Self::Circle),
            "square" => Some(Self::Square),
            _ => None,
        }
    }
}

/// The active workspace's pill: logical pixels. The default is a square
/// tile: a `square` with radius 3, lifted 6 off the bar's top and bottom,
/// its span grown to the tile ([`super::Settings::disc`]), with three
/// spaces between the numbers ([`super::DEFAULT_ITEM_GAP`]). At the
/// default bar (28 tall, DejaVu Sans 14, padding 8) the tile is about
/// 18 wide and 18 tall: as wide as the bar's inner height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pill {
    pub shape: Shape,
    /// The corner radius of a `rect`, cut back to half the pill's shorter
    /// side (the other shapes have the largest radius that fits).
    pub radius: u32,
    /// The gap from the bar's top and bottom edges.
    pub inset: u32,
}

impl Default for Pill {
    fn default() -> Self {
        Self {
            shape: Shape::Square,
            radius: 3,
            inset: 6,
        }
    }
}

impl Pill {
    /// The pill's rows, `top .. bottom`, on a bar `height` device pixels
    /// tall whose text line is `line` tall.
    pub fn rows(&self, height: u32, line: u32, scale: Scale) -> (u32, u32) {
        let room = height.saturating_sub(line) / 2;
        let inset = device(self.inset, scale).min(room);
        (inset, height - inset)
    }

    /// The corner radius in device pixels, before the fill cuts it back to
    /// what fits.
    pub fn radius(&self, scale: Scale) -> u32 {
        match self.shape {
            Shape::Rect | Shape::Square => device(self.radius, scale),
            Shape::Pill | Shape::Circle => u32::MAX,
        }
    }

    /// The pill's horizontal extent `(lo, hi)`, from the item's padded
    /// `natural` extent, `rows` tall, within `room` (which holds `natural`:
    /// what a forced shape's growth may not pass). All device pixels, one
    /// origin. A `circle` or `square` grows to at least the rows' height.
    pub fn extent(&self, natural: (u32, u32), room: (u32, u32), rows: (u32, u32)) -> (u32, u32) {
        if !matches!(self.shape, Shape::Circle | Shape::Square) {
            return natural;
        }
        let width = natural.1.saturating_sub(natural.0);
        let diameter = rows.1.saturating_sub(rows.0);
        let want = width.max(diameter);
        let room_width = room.1.saturating_sub(room.0);
        let got = want.min(room_width);
        if got <= width {
            return natural;
        }
        // Centered on the number, then slid back into the room.
        let center = (u64::from(natural.0) + u64::from(natural.1)) / 2;
        let lo = center.saturating_sub(u64::from(got / 2));
        let lo = lo.clamp(u64::from(room.0), u64::from(room.1.saturating_sub(got))) as u32;
        (lo, lo + got)
    }
}
