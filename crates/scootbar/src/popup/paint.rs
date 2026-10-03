//! Painting a popup into its buffer: a frame, then each widget in its row,
//! in the bar's own theme tokens. The whole popup is drawn each time, as it
//! is a few thousand pixels, into the pooled buffer, with no allocation.

use super::interact::Interaction;
use super::layout::Layout;
use super::{Content, Kind};
use crate::paint::{Canvas, Span};
use crate::text::Text;
use crate::theme::Theme;

/// Draws `content` as `layout` places it. `em` is the em in device pixels
/// (what `layout` was computed at), `interaction` says what is hovered and
/// what a drag shows. A canvas smaller than the layout is clipped, never a
/// panic.
pub fn paint(
    canvas: &mut Canvas<'_>,
    text: &mut Text,
    theme: &Theme,
    content: &Content,
    layout: &Layout,
    interaction: &Interaction,
    em: f32,
) {
    let (width, height) = (layout.width, layout.height);
    let whole = Span { x: 0, width };
    // The frame is the `dim` token, the inside the bar's background.
    canvas.fill_rect(whole, 0, height, theme.dim);
    let frame = layout.frame;
    let inside = Span {
        x: frame,
        width: width.saturating_sub(frame.saturating_mul(2)),
    };
    canvas.fill_rect(
        inside,
        frame,
        height.saturating_sub(frame),
        theme.background,
    );
    let clip = Span {
        x: frame.saturating_add(layout.pad),
        width: width
            .saturating_sub(frame.saturating_mul(2))
            .saturating_sub(layout.pad.saturating_mul(2)),
    };
    let metrics = text.metrics(em);
    for (index, (widget, row)) in content.widgets().iter().zip(layout.rows()).enumerate() {
        let label = content.label(widget);
        let baseline = i64::from(row.y0) + metrics.baseline(row.y1.saturating_sub(row.y0));
        match widget.kind {
            Kind::Text => {
                text.draw(
                    canvas,
                    None,
                    label,
                    em,
                    i64::from(clip.x),
                    baseline,
                    theme.foreground,
                    clip,
                );
            }
            Kind::Button { selected, .. } => {
                if interaction.hover() == Some(index) {
                    canvas.fill_rect(inside, row.y0, row.y1, theme.dim);
                }
                let color = if selected {
                    theme.accent
                } else {
                    theme.foreground
                };
                text.draw(
                    canvas,
                    None,
                    label,
                    em,
                    i64::from(clip.x),
                    baseline,
                    color,
                    clip,
                );
            }
            Kind::Slider { value, max, .. } => {
                let shown = match interaction.dragging() {
                    Some((dragged, value)) if dragged == index => value,
                    _ => value,
                };
                slider(canvas, theme, layout, index, shown, max);
            }
        }
    }
}

/// A slider's track (filled `accent` up to the knob, `dim` after) and knob.
fn slider(
    canvas: &mut Canvas<'_>,
    theme: &Theme,
    layout: &Layout,
    index: usize,
    value: u32,
    max: u32,
) {
    let Some(row) = layout.rows().get(index).copied() else {
        return;
    };
    let track = layout.track(index);
    if track.width == 0 {
        return;
    }
    let knob = layout.knob(row).max(2);
    let thick = (knob / 3).max(2);
    let middle = row.y0 + (row.y1.saturating_sub(row.y0)) / 2;
    let y0 = middle.saturating_sub(thick / 2);
    let y1 = y0.saturating_add(thick);
    let at = layout.knob_x(index, value, max);
    let rail = Span {
        x: track.x,
        width: track.width,
    };
    canvas.fill_pill(rail, y0, y1, thick / 2, theme.dim);
    let filled = Span {
        x: track.x,
        width: at.saturating_sub(track.x),
    };
    if filled.width > 0 {
        canvas.fill_pill(filled, y0, y1, thick / 2, theme.accent);
    }
    let left = at.saturating_sub(knob / 2);
    let top = middle.saturating_sub(knob / 2);
    canvas.fill_pill(
        Span {
            x: left,
            width: knob,
        },
        top,
        top.saturating_add(knob),
        knob / 2,
        theme.foreground,
    );
}
