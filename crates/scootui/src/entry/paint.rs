//! Drawing the entry, list and selector into the caller's canvas, in
//! the bar's theme tokens. The whole widget is drawn each time (a few
//! thousand pixels), into the surface's pooled buffer, with no
//! allocation: text draws clipped to the canvas, so overlong lines cost
//! resolve time, never memory.
//!
//! Token mapping (the look contract's roles resolve onto these when the
//! apps theme through it; the defaults are Catppuccin Mocha-ish):
//!
//! - text, caret-side ink: `foreground`; placeholder and hints: `dim`;
//! - caret bar and focus ring: `accent`, 2 device pixels;
//! - select-all and the selected row: `accent` fill with `background` ink;
//! - match-highlight spans: `hover` on plain rows; on the selected row
//!   [`Theme::match_on_selection`] (legible `hover` when it contrasts
//!   with the fill, else the inverted chip, else black or white).

use super::line::{Entry, MASK_CHAR, PasswordEntry};
use super::list::{List, Selector};
use crate::paint::{Canvas, Span};
use crate::text::Text;
use crate::theme::{SelectedMatch, Theme};

/// Where a widget draws: top-left (`x`, `y`) in device pixels, `width`
/// wide, each row `row_height` tall, text at `em`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub row_height: u32,
    pub em: f32,
}

impl Geometry {
    /// `None` when there is nothing to draw into (zero width or zero
    /// row height): the paints draw nothing rather than dividing by
    /// zero or underflowing.
    fn usable(self) -> bool {
        self.width > 0 && self.row_height > 0
    }

    /// The gap between a widget's edge (ring or selection fill) and its
    /// text: half an em, so the first letter never touches the edge.
    fn inset(self) -> u32 {
        (self.em * 0.5).round().clamp(0.0, 64.0) as u32
    }

    /// Where text starts: callers drawing beside the widgets use this to
    /// stay aligned with them.
    pub fn text_x(self) -> u32 {
        self.x.saturating_add(self.inset())
    }

    /// The text's clip: the widget minus the inset on both sides.
    pub fn clip(self) -> Span {
        Span {
            x: self.text_x(),
            width: self.width.saturating_sub(self.inset().saturating_mul(2)),
        }
    }
}

/// Draw one entry line: the text (or `placeholder` in `dim` when
/// empty), the caret when focused, and the focus ring when focused.
#[allow(clippy::too_many_arguments)]
pub fn paint_entry(
    canvas: &mut Canvas<'_>,
    text: &mut Text,
    theme: &Theme,
    entry: &Entry,
    geo: &Geometry,
    focused: bool,
    placeholder: &str,
) {
    if !geo.usable() {
        return;
    }
    let clip = geo.clip();
    let baseline = i64::from(geo.y) + text.metrics(geo.em).baseline(geo.row_height);
    if entry.is_empty() {
        text.draw(
            canvas,
            None,
            placeholder,
            geo.em,
            i64::from(geo.text_x()),
            baseline,
            theme.dim,
            clip,
        );
    } else if entry.selected_all() {
        let line = entry.text();
        let wide = text.measure(None, line, geo.em);
        canvas.fill_rect(
            Span {
                x: geo.text_x(),
                width: wide,
            },
            geo.y,
            geo.y.saturating_add(geo.row_height),
            theme.accent,
        );
        text.draw(
            canvas,
            None,
            line,
            geo.em,
            i64::from(geo.text_x()),
            baseline,
            theme.background,
            clip,
        );
    } else {
        let line = entry.text();
        text.draw(
            canvas,
            None,
            line,
            geo.em,
            i64::from(geo.text_x()),
            baseline,
            theme.foreground,
            clip,
        );
        if focused {
            let prefix = line.get(..entry.caret().min(line.len())).unwrap_or("");
            let at = geo
                .text_x()
                .saturating_add(text.measure(None, prefix, geo.em));
            canvas.fill_rect(
                Span { x: at, width: 2 },
                geo.y.saturating_add(1),
                geo.y.saturating_add(geo.row_height).saturating_sub(1),
                theme.accent,
            );
        }
    }
    if focused {
        focus_ring(canvas, theme, geo.x, geo.y, geo.width, geo.row_height);
    }
}

/// Draw the password entry: one [`MASK_CHAR`](super::line::MASK_CHAR)
/// per character (never the text), the caret when focused, and the
/// focus ring when focused. The mask is drawn character by character
/// from a one-byte literal, so no mask-sized string is ever built.
pub fn paint_password(
    canvas: &mut Canvas<'_>,
    text: &mut Text,
    theme: &Theme,
    entry: &PasswordEntry<'_>,
    geo: &Geometry,
    focused: bool,
) {
    if !geo.usable() {
        return;
    }
    const MASK: &str = "*";
    debug_assert_eq!(MASK_CHAR, '*');
    let clip = geo.clip();
    let baseline = i64::from(geo.y) + text.metrics(geo.em).baseline(geo.row_height);
    let color = if entry.selected_all() {
        theme.background
    } else {
        theme.foreground
    };
    if entry.selected_all() {
        let wide = (entry.char_len() as f32 * text.advance(MASK_CHAR, geo.em))
            .ceil()
            .max(0.0) as u32;
        canvas.fill_rect(
            Span {
                x: geo.text_x(),
                width: wide,
            },
            geo.y,
            geo.y.saturating_add(geo.row_height),
            theme.accent,
        );
    }
    let advance = text.advance(MASK_CHAR, geo.em);
    let mut pen = geo.text_x() as f32;
    let stop = clip.end() as f32;
    for _ in 0..entry.char_len() {
        if pen >= stop {
            break;
        }
        text.draw(
            canvas,
            None,
            MASK,
            geo.em,
            pen.round() as i64,
            baseline,
            color,
            clip,
        );
        pen += advance;
    }
    if focused && !entry.selected_all() {
        let mut caret = geo.text_x() as f32;
        for _ in 0..entry.caret_chars() {
            caret += advance;
        }
        canvas.fill_rect(
            Span {
                x: caret.round().max(0.0) as u32,
                width: 2,
            },
            geo.y.saturating_add(1),
            geo.y.saturating_add(geo.row_height).saturating_sub(1),
            theme.accent,
        );
    }
    if focused {
        focus_ring(canvas, theme, geo.x, geo.y, geo.width, geo.row_height);
    }
}

/// Draw the list's visible window: exactly
/// [`List::visible_range`] rows, read through `row_text`, each with
/// its match-highlight spans. Rows outside the window are never read,
/// so a 65,536-row index draws like a 16-row one.
#[allow(clippy::too_many_arguments)]
pub fn paint_list<'a, 'b>(
    canvas: &mut Canvas<'_>,
    text: &mut Text,
    theme: &Theme,
    list: &List,
    geo: &Geometry,
    row_text: &'b (dyn Fn(usize) -> &'a str + 'b),
    focused: bool,
) {
    if !geo.usable() {
        return;
    }
    paint_rows(canvas, text, theme, list, geo, geo.y, row_text);
    if focused {
        let rows = list.visible_range().len() as u32;
        let tall = rows.saturating_mul(geo.row_height);
        focus_ring(
            canvas,
            theme,
            geo.x,
            geo.y,
            geo.width,
            tall.max(geo.row_height),
        );
    }
}

/// Draw the selector: the collapsed line (the choice, or dim `empty`
/// text when there is none), then, when expanded, the rows below it
/// exactly as [`paint_list`] draws them.
#[allow(clippy::too_many_arguments)]
pub fn paint_selector<'a, 'b>(
    canvas: &mut Canvas<'_>,
    text: &mut Text,
    theme: &Theme,
    selector: &Selector,
    geo: &Geometry,
    row_text: &'b (dyn Fn(usize) -> &'a str + 'b),
    empty: &str,
    focused: bool,
) {
    if !geo.usable() {
        return;
    }
    let clip = geo.clip();
    let baseline = i64::from(geo.y) + text.metrics(geo.em).baseline(geo.row_height);
    match selector.selected() {
        Some(index) => {
            text.draw(
                canvas,
                None,
                row_text(index),
                geo.em,
                i64::from(geo.text_x()),
                baseline,
                theme.foreground,
                clip,
            );
        }
        None => {
            text.draw(
                canvas,
                None,
                empty,
                geo.em,
                i64::from(geo.text_x()),
                baseline,
                theme.dim,
                clip,
            );
        }
    }
    if focused {
        focus_ring(canvas, theme, geo.x, geo.y, geo.width, geo.row_height);
    }
    if selector.is_expanded() {
        paint_rows(
            canvas,
            text,
            theme,
            selector.list(),
            geo,
            geo.y.saturating_add(geo.row_height),
            row_text,
        );
    }
}

/// The rows `list` shows, one [`Geometry::row_height`] each from `y0`.
/// Shared by the list and the expanded selector: one code path means
/// one golden covers both.
fn paint_rows<'a, 'b>(
    canvas: &mut Canvas<'_>,
    text: &mut Text,
    theme: &Theme,
    list: &List,
    geo: &Geometry,
    y0: u32,
    row_text: &'b (dyn Fn(usize) -> &'a str + 'b),
) {
    let clip = geo.clip();
    let metrics = text.metrics(geo.em);
    let selected = list.selected();
    // Resolved once for the whole window: pure contrast math, no
    // allocation, so the per-row loop stays allocation-free.
    let on_selection = theme.match_on_selection();
    for (slot, row) in list.visible_range().enumerate() {
        let slot = slot as u32;
        let y = y0.saturating_add(slot.saturating_mul(geo.row_height));
        let label = row_text(row);
        let baseline = i64::from(y) + metrics.baseline(geo.row_height);
        if selected == Some(row) {
            canvas.fill_rect(
                Span {
                    x: geo.x,
                    width: geo.width,
                },
                y,
                y.saturating_add(geo.row_height),
                theme.accent,
            );
        }
        let base = if selected == Some(row) {
            theme.background
        } else {
            theme.foreground
        };
        text.draw(
            canvas,
            None,
            label,
            geo.em,
            i64::from(geo.text_x()),
            baseline,
            base,
            clip,
        );
        let is_selected = selected == Some(row);
        let match_ink = if is_selected {
            match on_selection {
                SelectedMatch::Ink(ink) => ink,
                // The inverted chip below fills each span's box, so the
                // ink here only matters for the non-chip arms.
                SelectedMatch::Inverted => theme.accent,
            }
        } else {
            theme.hover
        };
        for span in list.highlights_for(row) {
            let len = label.len() as u32;
            let lo = span.0.min(len);
            let hi = span.1.min(len);
            if lo >= hi {
                continue;
            }
            // Byte offsets into UTF-8: a span edge inside a character
            // draws nothing rather than panicking the frame.
            let (head, body) = match (
                label.get(..lo as usize),
                label.get(lo as usize..hi as usize),
            ) {
                (Some(head), Some(body)) => (head, body),
                _ => continue,
            };
            let at = geo
                .text_x()
                .saturating_add(text.measure(None, head, geo.em));
            if is_selected && matches!(on_selection, SelectedMatch::Inverted) {
                // The inverted chip: the row's text ink behind the span,
                // `accent` over it. Same pair as the selected row's own
                // text, so the highlight stays both visible and legible.
                let wide = text.measure(None, body, geo.em);
                canvas.fill_rect(
                    Span { x: at, width: wide },
                    y,
                    y.saturating_add(geo.row_height),
                    theme.background,
                );
            }
            text.draw(
                canvas,
                None,
                body,
                geo.em,
                i64::from(at),
                baseline,
                match_ink,
                clip,
            );
        }
    }
}

/// A 2-device-pixel `accent` outline around (`x`, `y`, `width`,
/// `height`): what Tab/Shift+Tab moves. Four clipped fills, so a
/// widget at the canvas edge rings its visible sides without panic.
fn focus_ring(canvas: &mut Canvas<'_>, theme: &Theme, x: u32, y: u32, width: u32, height: u32) {
    let y1 = y.saturating_add(height);
    canvas.fill_rect(Span { x, width }, y, y.saturating_add(2), theme.accent);
    canvas.fill_rect(Span { x, width }, y1.saturating_sub(2), y1, theme.accent);
    canvas.fill_rect(Span { x, width: 2 }, y, y1, theme.accent);
    canvas.fill_rect(
        Span {
            x: x.saturating_add(width).saturating_sub(2),
            width: 2,
        },
        y,
        y1,
        theme.accent,
    );
}
