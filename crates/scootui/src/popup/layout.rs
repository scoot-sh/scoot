//! Where each widget sits: rows stacked top to bottom inside a one-pixel
//! frame, sized from the em, so every length scales with the output.

use super::{Content, Kind};
use crate::paint::Span;
use crate::text::Text;

/// A row's height, in ems.
const ROW_EM: f32 = 1.9;
/// The narrowest a popup is, in ems.
const MIN_WIDTH_EM: f32 = 13.0;
/// The widest label counted toward the width, in ems: a longer one is cut
/// by the paint's clip, so one long name cannot make a popup wider than a
/// bar.
const MAX_LABEL_EM: f32 = 40.0;

/// A tooltip's row height, in ems: tighter than a popup's, which holds a
/// target to hit.
const TIP_ROW_EM: f32 = 1.4;
/// The widest a tooltip's text is wrapped to, in ems.
pub const TIP_MAX_EM: f32 = 30.0;

/// One row: `y0 .. y1` in device pixels, full width inside the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub y0: u32,
    pub y1: u32,
}

/// The popup's size and its rows, in device pixels at one scale.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    /// The frame's thickness, and the gap between the text and the frame's
    /// inside edge (one padding).
    pub frame: u32,
    pub pad: u32,
    rows: Vec<Row>,
}

impl Layout {
    /// Lays `content` out at `em` device pixels per em, `pad` device pixels
    /// of padding and a `frame` pixels frame. Reuses the row storage.
    /// Measures labels with `text`, which only reads its font. An empty
    /// content is a frame and nothing in it.
    pub fn compute(&mut self, content: &Content, text: &Text, em: f32, pad: u32, frame: u32) {
        let em = if em.is_finite() && em > 0.0 { em } else { 1.0 };
        let row = (em * ROW_EM).ceil().max(1.0) as u32;
        let cap = (em * MAX_LABEL_EM) as u32;
        let mut widest = (em * MIN_WIDTH_EM).ceil() as u32;
        for widget in content.widgets() {
            let label = content.label(widget);
            if label.is_empty() {
                continue;
            }
            let wanted = text.measure(None, label, em).min(cap);
            widest = widest.max(wanted.saturating_add(pad.saturating_mul(2)));
        }
        self.frame = frame;
        self.pad = pad;
        self.rows.clear();
        // Half a padding above the first row and below the last.
        let edge = frame.saturating_add(pad / 2);
        let mut y = edge;
        for _ in content.widgets() {
            let y1 = y.saturating_add(row);
            self.rows.push(Row { y0: y, y1 });
            y = y1;
        }
        self.height = y.saturating_add(edge);
        self.width = widest.saturating_add(frame.saturating_mul(2));
    }

    /// Lays a tooltip's lines out: as wide as the widest line (wrapped at
    /// [`TIP_MAX_EM`] by the caller) and padded, rows tighter than a
    /// popup's, no minimum width.
    pub fn compute_tooltip(
        &mut self,
        content: &Content,
        text: &Text,
        em: f32,
        pad: u32,
        frame: u32,
    ) {
        let em = if em.is_finite() && em > 0.0 { em } else { 1.0 };
        let row = (em * TIP_ROW_EM).ceil().max(1.0) as u32;
        let widest = content
            .widgets()
            .iter()
            .map(|widget| text.measure(None, content.label(widget), em))
            .max()
            .unwrap_or(0);
        self.frame = frame;
        self.pad = pad;
        self.rows.clear();
        let edge = frame.saturating_add(pad / 2);
        let mut y = edge;
        for _ in content.widgets() {
            let y1 = y.saturating_add(row);
            self.rows.push(Row { y0: y, y1 });
            y = y1;
        }
        self.height = y.saturating_add(edge);
        self.width = widest
            .saturating_add(pad.saturating_mul(2))
            .saturating_add(frame.saturating_mul(2));
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// How tall the content is, in device pixels: the last row and the
    /// edge below it. Past the visible height (what a configure smaller
    /// than asked leaves) the popup scrolls to it.
    pub fn total_height(&self) -> u32 {
        let edge = self.frame.saturating_add(self.pad / 2);
        match self.rows.last() {
            Some(row) => row.y1.saturating_add(edge),
            None => self.height,
        }
    }

    /// How far down the content may scroll, in device pixels: the content
    /// past the visible height, or zero where it all fits.
    pub fn max_scroll(&self) -> u32 {
        self.total_height().saturating_sub(self.height)
    }

    /// The widget under `(x, y)` (device pixels in the popup), if any.
    pub fn hit(&self, x: i64, y: i64) -> Option<usize> {
        let inside = x >= i64::from(self.frame)
            && x < i64::from(self.width) - i64::from(self.frame)
            && y >= 0;
        if !inside {
            return None;
        }
        self.rows
            .iter()
            .position(|row| y >= i64::from(row.y0) && y < i64::from(row.y1))
    }

    /// The slider track's horizontal extent in row `index`: the knob's
    /// center runs from `.x` to `.end()`. Empty when the popup is too
    /// narrow for a track.
    pub fn track(&self, index: usize) -> Span {
        let Some(row) = self.rows.get(index) else {
            return Span::default();
        };
        let knob = self.knob(*row);
        let x0 = self.frame.saturating_add(self.pad).saturating_add(knob / 2);
        let x1 = self
            .width
            .saturating_sub(self.frame)
            .saturating_sub(self.pad)
            .saturating_sub(knob / 2);
        Span {
            x: x0,
            width: x1.saturating_sub(x0),
        }
    }

    /// The knob's diameter in a row.
    pub fn knob(&self, row: Row) -> u32 {
        row.y1.saturating_sub(row.y0) * 3 / 5
    }

    /// The value a pointer at `x` means on the slider in row `index`, of
    /// `max`: the nearest whole step, held to `0..=max`.
    pub fn value_at(&self, index: usize, x: i64, max: u32) -> u32 {
        let track = self.track(index);
        if track.width == 0 {
            return 0;
        }
        let along = (x - i64::from(track.x)).clamp(0, i64::from(track.width)) as u64;
        let width = u64::from(track.width);
        // Rounded to nearest: `max` is at most `u32::MAX`, the product fits.
        ((along * u64::from(max) + width / 2) / width) as u32
    }

    /// The pixel a knob at `value` of `max` is centered on in row `index`.
    pub fn knob_x(&self, index: usize, value: u32, max: u32) -> u32 {
        let track = self.track(index);
        let max = u64::from(max.max(1));
        let along = u64::from(track.width) * u64::from(value).min(max) / max;
        track.x.saturating_add(along as u32)
    }

    /// Whether the widget in row `index` is a slider, with its `max`.
    pub fn slider_max(content: &Content, index: usize) -> Option<(u32, &'static str)> {
        match content.widgets().get(index)?.kind {
            Kind::Slider { max, action, .. } => Some((max, action)),
            _ => None,
        }
    }
}
