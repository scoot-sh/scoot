//! The input region's bookkeeping: the rectangles the cards' shapes cover,
//! rebuilt into pooled lists, and whether the compositor already has them.
//!
//! A `wl_region` is a protocol object (a request to create it, one per
//! rectangle, a destroy), so a draw that leaves the shape as it was sends
//! none of it. At rest that is every draw but the first after a change of
//! layout, look or config; in motion the rectangles move every frame and
//! each frame sends its own.

use scootui::region::{self, Rect};

use crate::view::layout::Card;

/// The rectangles to send, and the ones last sent.
#[derive(Debug, Default)]
pub(super) struct InputRegion {
    rects: Vec<Rect>,
    sent: Vec<Rect>,
    /// Whether this surface has been given a region at all. A surface
    /// without one takes input everywhere, so an empty list is still a
    /// region to send the first time.
    set: bool,
}

impl InputRegion {
    /// The list to fill for this draw, emptied; capacity is kept.
    pub(super) fn begin(&mut self) -> &mut Vec<Rect> {
        self.rects.clear();
        &mut self.rects
    }

    /// Whether the list built since [`begin`](Self::begin) differs from
    /// what the surface has.
    pub(super) fn stale(&self) -> bool {
        !self.set || self.rects != self.sent
    }

    /// The list built since [`begin`](Self::begin).
    pub(super) fn rects(&self) -> &[Rect] {
        &self.rects
    }

    /// Records the built list as sent.
    pub(super) fn sent(&mut self) {
        std::mem::swap(&mut self.rects, &mut self.sent);
        self.set = true;
    }

    /// The surface is gone: its successor has no region yet.
    pub(super) fn forget(&mut self) {
        self.set = false;
    }
}

/// Appends the rectangles of `card`'s outline where the layout put it,
/// corners cut to `radius`.
pub(super) fn push_at_rest(card: &Card, radius: u32, out: &mut Vec<Rect>) {
    let rect = card.rect;
    let start = out.len();
    region::push_input_rects(rect.width, rect.height, radius, out);
    for row in &mut out[start..] {
        row.x = row.x.saturating_add(rect.x);
        row.y = row.y.saturating_add(rect.y);
    }
    // A pill's middle band can be empty: the wire has no use for it.
    let mut kept = start;
    for read in start..out.len() {
        if out[read].width > 0 && out[read].height > 0 {
            out[kept] = out[read];
            kept += 1;
        }
    }
    out.truncate(kept);
}
