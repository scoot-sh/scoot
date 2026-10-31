//! The apps panel's geometry: one place that says where the search
//! field, the rows, the selection highlight and the icon column sit.
//!
//! Everything derives from the look's font size (`em`) and the panel's
//! corner radius, in the pixels of whatever scale it is built for, so a
//! 20 pt font or a 2x output lays out the same shape as 13 pt at 1x:
//!
//! - one inner padding ([`Layout::inner`]) from the panel edge to the
//!   search field and every row, so their left and right edges line up;
//! - the selection highlight spans the whole row ([`Layout::row`]) with
//!   the panel's corner radius, and is drawn first;
//! - the icon column sits *inside* the highlight, one [`Layout::lead`]
//!   from its left edge, so the highlight never touches an icon;
//! - the text starts after the icon column and stops one `lead` short of
//!   the row's right edge.
//!
//! Pure integer geometry with no I/O, so the invariants are unit-tested.

use crate::icons::MAX_RENDER_SIDE;
use scootui::entry::Geometry;

/// The panel border, in logical pixels (scaled with the output).
pub const BORDER: u32 = 2;

/// The dmenu panel's padding, list gap and the room a row adds to the
/// text's height, in logical pixels: it keeps fixed spacing, which scales
/// with the output.
pub const DMENU_PAD: u32 = 8;
pub const DMENU_GAP: u32 = 4;
pub const DMENU_ROW_SLACK: u32 = 8;

/// A pixel rectangle: top-left corner and size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn right(self) -> u32 {
        self.x.saturating_add(self.width)
    }
}

#[cfg(test)]
impl Rect {
    pub fn bottom(self) -> u32 {
        self.y.saturating_add(self.height)
    }

    /// Whether the two share any pixel.
    pub fn overlaps(self, other: Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// Whether `other` lies wholly inside `self`.
    pub fn contains(self, other: Rect) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }
}

/// The panel's measurements in device pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    /// The text size these were derived from.
    pub em: f32,
    pub border: u32,
    /// Padding between the border and the field and rows.
    pub pad: u32,
    /// Vertical gap between the search field and the first row.
    pub gap: u32,
    pub row_h: u32,
    /// The icon's side.
    pub side: u32,
    /// Space between the icon column and the row text.
    pub icon_gap: u32,
    /// From the highlight's edge to the icon, and from the text's end to
    /// the highlight's right edge.
    pub lead: u32,
    /// The highlight's corner radius.
    pub radius: u32,
}

fn round(value: f32) -> u32 {
    value.round().clamp(0.0, 4096.0) as u32
}

impl Layout {
    /// The layout for text `em` pixels tall on a panel of corner `radius`,
    /// at output scale `factor` (1.0 for logical pixels). `em` and
    /// `radius` are already in device pixels.
    pub fn new(em: f32, radius: u32, factor: f64) -> Self {
        let em = if em.is_finite() {
            em.clamp(1.0, 512.0)
        } else {
            1.0
        };
        let factor = if factor.is_finite() {
            factor.clamp(0.25, 16.0)
        } else {
            1.0
        };
        let radius = radius.min(4096);
        let border = ((f64::from(BORDER) * factor).round() as u32).max(1);
        // Far enough in that square field corners clear the rounded
        // corner: a circle's corner cut is r (1 - 1/sqrt 2) = 0.293 r.
        let pad = round(em * 0.75).max((radius * 3).div_ceil(10)).max(1);
        let gap = round(em * 0.4).max(1);
        let row_h = (em * 1.75).ceil().min(4096.0) as u32;
        let side = round(em * 1.2)
            .clamp(1, row_h.saturating_sub(4).max(1))
            .min(MAX_RENDER_SIDE);
        let icon_gap = round(em * 0.5).max(1);
        let radius = radius.min(row_h / 2);
        let lead = round(em * 0.6).max(radius.div_ceil(2)).max(1);
        Self {
            em,
            border,
            pad,
            gap,
            row_h,
            side,
            icon_gap,
            lead,
            radius,
        }
    }

    /// The dmenu panel's geometry at scale 1: no icons, rows hugging the
    /// text.
    pub fn dmenu(em: f32) -> Self {
        Self::dmenu_scaled(em, 1.0)
    }

    /// The dmenu panel's geometry at output scale `factor`: `em` is in
    /// device pixels, and the border, padding and gap (fixed logical
    /// numbers) scale with the output so a 2x output keeps the shape. At
    /// `factor` 1.0 these are the numbers the panel always had.
    pub fn dmenu_scaled(em: f32, factor: f64) -> Self {
        let em = if em.is_finite() {
            em.clamp(1.0, 512.0)
        } else {
            1.0
        };
        let factor = if factor.is_finite() {
            factor.clamp(0.25, 16.0)
        } else {
            1.0
        };
        let scaled = |logical: u32| (f64::from(logical) * factor).round() as u32;
        Self {
            em,
            border: scaled(BORDER).max(1),
            pad: scaled(DMENU_PAD),
            gap: scaled(DMENU_GAP),
            row_h: (em.ceil() as u32).saturating_add(scaled(DMENU_ROW_SLACK)),
            side: 0,
            icon_gap: 0,
            lead: 0,
            radius: 0,
        }
    }

    /// From the panel's edge to the field and rows.
    pub fn inner(&self) -> u32 {
        self.border.saturating_add(self.pad)
    }

    /// The panel's height for `rows` list rows.
    pub fn height(&self, rows: u32) -> u32 {
        self.inner()
            .saturating_mul(2)
            .saturating_add(self.row_h.saturating_mul(rows.saturating_add(1)))
            .saturating_add(self.gap)
    }

    /// The box inside the border and padding that holds field and rows.
    pub fn inner_box(&self, width: u32, rows: u32) -> Rect {
        let inner = self.inner();
        Rect {
            x: inner,
            y: inner,
            width: width.saturating_sub(inner.saturating_mul(2)),
            height: self
                .row_h
                .saturating_mul(rows.saturating_add(1))
                .saturating_add(self.gap),
        }
    }

    /// The search field.
    pub fn field(&self, width: u32) -> Rect {
        Rect {
            height: self.row_h,
            ..self.inner_box(width, 0)
        }
    }

    /// The list's top, below the field and its gap.
    pub fn list_y(&self) -> u32 {
        self.inner()
            .saturating_add(self.row_h)
            .saturating_add(self.gap)
    }

    /// List row `slot` (0 is the first visible one): the selection
    /// highlight's rectangle.
    pub fn row(&self, width: u32, slot: u32) -> Rect {
        Rect {
            y: self
                .list_y()
                .saturating_add(slot.saturating_mul(self.row_h)),
            height: self.row_h,
            ..self.inner_box(width, 0)
        }
    }

    /// The icon square of `row`, vertically centered.
    pub fn icon(&self, row: Rect) -> Rect {
        Rect {
            x: row.x.saturating_add(self.lead),
            y: row
                .y
                .saturating_add(self.row_h.saturating_sub(self.side) / 2),
            width: self.side,
            height: self.side,
        }
    }

    /// Where `row`'s text starts, after the icon column.
    pub fn text_x(&self, row: Rect) -> u32 {
        self.icon(row).right().saturating_add(self.icon_gap)
    }

    /// The right end of `row`'s text: one `lead` inside the highlight.
    pub fn text_end(&self, row: Rect) -> u32 {
        row.right().saturating_sub(self.lead)
    }

    /// The search field's widget geometry. Its text starts half an em in,
    /// as the shared widgets place it.
    pub fn field_geometry(&self, width: u32) -> Geometry {
        let field = self.field(width);
        Geometry {
            x: field.x,
            y: field.y,
            width: field.width,
            row_height: field.height,
            em: self.em,
        }
    }

    /// The list's widget geometry for `rows` laid out from the first
    /// row: text at [`Layout::text_x`], clipped at [`Layout::text_end`].
    pub fn list_geometry(&self, width: u32) -> Geometry {
        let row = self.row(width, 0);
        let inset = widget_inset(self.em);
        let start = self.text_x(row);
        let clip = self.text_end(row).saturating_sub(start);
        Geometry {
            x: start.saturating_sub(inset),
            y: row.y,
            width: clip.saturating_add(inset.saturating_mul(2)),
            row_height: self.row_h,
            em: self.em,
        }
    }
}

/// The shared widgets' text inset (half an em): [`Geometry::text_x`]
/// adds it to the geometry's x.
fn widget_inset(em: f32) -> u32 {
    (em * 0.5).round().clamp(0.0, 64.0) as u32
}

#[cfg(test)]
mod tests;
