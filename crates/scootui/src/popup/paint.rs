//! Painting a popup into its buffer: a frame, then each widget in its row,
//! in the bar's own theme tokens. The whole popup is drawn each time, as it
//! is a few thousand pixels, into the pooled buffer, with no allocation.

use super::interact::Interaction;
use super::layout::{Layout, Row};
use super::{Content, Kind, MAX_TEXT, Round};
use crate::paint::Shadow;
use crate::paint::{Canvas, Span};
use crate::region::Rect;
use crate::text::Text;
use crate::theme::Theme;

/// Draws `content` as `layout` places it. `em` is the em in device pixels
/// (what `layout` was computed at), `interaction` says what is hovered and
/// what a drag shows, `round` is the frame's shape: square corners paint as
/// before (two opaque fills), rounded ones as a border ring over a
/// transparent buffer, with the content clipped back to the inner arc after
/// it is drawn. A canvas smaller than the layout is clipped, never a
/// panic; rows past it (a list taller than the output, scrolled) draw with
/// an ellipsis where the window title's does, from a stack buffer, so the
/// draw allocates nothing either way.
#[allow(clippy::too_many_arguments)] // A paint entry point, as the bar's own is.
pub fn paint(
    canvas: &mut Canvas<'_>,
    text: &mut Text,
    theme: &Theme,
    content: &Content,
    layout: &Layout,
    interaction: &Interaction,
    em: f32,
    round: &Round,
) {
    let (width, height) = (layout.width, layout.height);
    let whole = Span { x: 0, width };
    if round.rounded() {
        // Transparent corners need an alpha channel: the daemon draws a
        // rounded popup into an `ARGB8888` buffer.
        canvas.clear();
        canvas.fill_rounded(0, 0, width, height, theme.dim, round.outer());
        let frame = round.frame();
        canvas.fill_rounded(
            frame,
            frame,
            width.saturating_sub(frame),
            height.saturating_sub(frame),
            theme.background,
            round.inner(),
        );
    } else {
        // The frame is the `dim` token, the inside the bar's background.
        canvas.fill_rect(whole, 0, height, theme.dim);
    }
    let frame = layout.frame;
    debug_assert!(!round.rounded() || round.frame() == frame);
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
    let scroll = interaction.scroll() as i64;
    let mut kept = [0u8; MAX_TEXT];
    for (index, (widget, row)) in content.widgets().iter().zip(layout.rows()).enumerate() {
        let label = content.label(widget);
        // Rows above or below the drawn part are skipped: the canvas clips
        // what is left either way, but a list taller than the output has
        // rows wholly off it.
        let y0 = i64::from(row.y0) - scroll;
        let y1 = i64::from(row.y1) - scroll;
        if y1 <= 0 || y0 >= i64::from(layout.height) {
            continue;
        }
        let baseline = y0 + metrics.baseline(row.y1.saturating_sub(row.y0));
        match widget.kind {
            Kind::Text => {
                let shown = cut(text, label, em, clip.width, &mut kept);
                text.draw(
                    canvas,
                    None,
                    shown,
                    em,
                    i64::from(clip.x),
                    baseline,
                    theme.foreground,
                    clip,
                );
            }
            Kind::Button { selected, .. } => {
                if interaction.hover() == Some(index) {
                    canvas.fill_rect(inside, y0.max(0) as u32, y1.max(0) as u32, theme.dim);
                }
                let color = if selected {
                    theme.accent
                } else {
                    theme.foreground
                };
                let shown = cut(text, label, em, clip.width, &mut kept);
                text.draw(
                    canvas,
                    None,
                    shown,
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
                slider(
                    canvas,
                    theme,
                    layout.track(index),
                    *row,
                    layout.knob_x(index, shown, max),
                    scroll,
                );
            }
        }
    }
    if round.rounded() {
        // What the rows painted past the inner arc (a hover fill, a glyph,
        // a slider's end) returns to the frame's own edge.
        canvas.restore_frame_edge(
            0,
            0,
            width,
            height,
            round.frame(),
            theme.dim,
            theme.background,
            round.outer(),
            round.inner(),
        );
    }
}

/// Draws `content` as `layout` places it, offset by `origin` (the popup's
/// top-left in the surface), with a drop shadow and a gradient fill.
///
/// `origin` is `(0, 0)` for an unshadowed popup (the surface is exactly the
/// popup); a shadowed popup's surface is larger by the shadow's pad on
/// every side (see [`Shadow::pad`]), and the popup sits at
/// `(pad.left, pad.top)`. `shadow` is the prepared tables, the tint
/// (black, or the accent for a glow) and the popup's corner radius;
/// `None` draws no shadow. `fill` is the top and bottom stops (equal for a
/// solid fill) and `opacity` the background's alpha.
///
/// The whole popup is drawn each time, into the pooled buffer, with no
/// allocation once the shadow tables are prepared. A square opaque solid
/// unshadowed popup at the origin paints exactly what [`paint`] does.
#[allow(clippy::too_many_arguments)]
pub fn paint_depth(
    canvas: &mut Canvas<'_>,
    text: &mut Text,
    theme: &Theme,
    content: &Content,
    layout: &Layout,
    interaction: &Interaction,
    em: f32,
    round: &Round,
    origin: (u32, u32),
    shadow: Option<(&Shadow, crate::color::Color, u32)>,
    fill: (crate::color::Color, crate::color::Color),
    opacity: u8,
) {
    let (ox, oy) = origin;
    let (width, height) = (layout.width, layout.height);
    let outer_rect = Rect {
        x: ox,
        y: oy,
        width,
        height,
    };
    let outer_radius = round.outer().radius();
    // Transparent corners, a shadow's falloff or a translucent fill need an
    // alpha channel: clear to transparent first. A square opaque fill
    // overwrites every pixel below.
    let need_alpha = shadow.is_some() || round.rounded() || opacity < 255 || fill.0 != fill.1;
    if need_alpha {
        canvas.clear();
    }
    if let Some((tables, tint, radius)) = shadow {
        tables.paint(canvas, outer_rect, radius, tint);
    }
    let frame = layout.frame;
    let (top_fill, bottom_fill) = fill;
    // The frame (the `dim` token) and the inside (the gradient stops) at
    // `opacity`. Equal stops are the solid path.
    canvas.fill_rounded_gradient(outer_rect, outer_radius, theme.dim, theme.dim, opacity);
    let inner_rect = Rect {
        x: ox.saturating_add(frame),
        y: oy.saturating_add(frame),
        width: width.saturating_sub(frame.saturating_mul(2)),
        height: height.saturating_sub(frame.saturating_mul(2)),
    };
    let inner_radius = round.inner().radius();
    if inner_rect.width > 0 && inner_rect.height > 0 {
        canvas.fill_rounded_gradient(inner_rect, inner_radius, top_fill, bottom_fill, opacity);
    }
    let inside = Span {
        x: ox.saturating_add(frame),
        width: width.saturating_sub(frame.saturating_mul(2)),
    };
    let clip = Span {
        x: ox.saturating_add(frame).saturating_add(layout.pad),
        width: width
            .saturating_sub(frame.saturating_mul(2))
            .saturating_sub(layout.pad.saturating_mul(2)),
    };
    let metrics = text.metrics(em);
    let scroll = interaction.scroll() as i64;
    let mut kept = [0u8; MAX_TEXT];
    for (index, (widget, row)) in content.widgets().iter().zip(layout.rows()).enumerate() {
        let label = content.label(widget);
        // Rows above or below the drawn part are skipped: the canvas clips
        // what is left either way, but a list taller than the output has
        // rows wholly off it. Rows are popup-local; the surface offset
        // follows.
        let y0 = i64::from(row.y0) - scroll + i64::from(oy);
        let y1 = i64::from(row.y1) - scroll + i64::from(oy);
        if y1 <= i64::from(oy) || y0 >= i64::from(oy) + i64::from(layout.height) {
            continue;
        }
        let baseline = y0 + metrics.baseline(row.y1.saturating_sub(row.y0));
        match widget.kind {
            Kind::Text => {
                let shown = cut(text, label, em, clip.width, &mut kept);
                text.draw(
                    canvas,
                    None,
                    shown,
                    em,
                    i64::from(clip.x),
                    baseline,
                    theme.foreground,
                    clip,
                );
            }
            Kind::Button { selected, .. } => {
                if interaction.hover() == Some(index) {
                    canvas.fill_rect(
                        Span {
                            x: inside.x,
                            width: inside.width,
                        },
                        y0.max(i64::from(oy)) as u32,
                        y1.max(i64::from(oy)) as u32,
                        theme.dim,
                    );
                }
                let color = if selected {
                    theme.accent
                } else {
                    theme.foreground
                };
                let shown = cut(text, label, em, clip.width, &mut kept);
                text.draw(
                    canvas,
                    None,
                    shown,
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
                // Tracks and knobs are popup-local; shift them into the
                // surface by the origin. Rows carry the origin; `dy` stays
                // the scroll, so `y0 = (row + oy) - scroll` lands in the
                // surface.
                let track = layout.track(index);
                let track = Span {
                    x: track.x.saturating_add(ox),
                    width: track.width,
                };
                let row_shifted = Row {
                    y0: row.y0.saturating_add(oy),
                    y1: row.y1.saturating_add(oy),
                };
                slider(
                    canvas,
                    theme,
                    track,
                    row_shifted,
                    layout.knob_x(index, shown, max).saturating_add(ox),
                    scroll,
                );
            }
        }
    }
    if round.rounded() {
        // What the rows painted past the inner arc returns to the frame's
        // own edge. For a gradient fill the inner edge is approximated
        // with the bottom stop; gradient goldens stay square, so no golden
        // pins the difference.
        canvas.restore_frame_edge(
            ox,
            oy,
            ox.saturating_add(width),
            oy.saturating_add(height),
            round.frame(),
            theme.dim,
            fill.1,
            round.outer(),
            round.inner(),
        );
    }
}

/// `label` cut to `available` device pixels at `em`, with an ellipsis where
/// the window title's is cut: the whole label where it fits, nothing but
/// the ellipsis where even that does not, else the longest prefix leaving
/// room for it. Written into `buf` (so the draw allocates nothing) and
/// lasting as long as it does.
pub(super) fn cut<'b>(
    text: &Text,
    label: &'b str,
    em: f32,
    available: u32,
    buf: &'b mut [u8; MAX_TEXT],
) -> &'b str {
    if text.measure(None, label, em) <= available {
        return label;
    }
    const ELLIPSIS: char = '…';
    let ellipsis = text.measure(None, "…", em);
    if ellipsis > available {
        return "";
    }
    let budget = available - ellipsis;
    let mut len = 0usize;
    let mut pen = 0.0f32;
    for c in label.chars().filter(|c| !c.is_control()) {
        let next = pen + text.advance(c, em);
        if next.round() > budget as f32 {
            break;
        }
        let width = c.len_utf8();
        if len + width + ELLIPSIS.len_utf8() > buf.len() {
            break;
        }
        c.encode_utf8(&mut buf[len..]);
        len += width;
        pen = next;
    }
    ELLIPSIS.encode_utf8(&mut buf[len..]);
    len += ELLIPSIS.len_utf8();
    std::str::from_utf8(&buf[..len]).unwrap_or("")
}

/// A slider's track (filled `accent` up to the knob, `dim` after) and knob.
/// `row` is the row as laid out (content coordinates) and `track` its
/// slider extent; `at` is the knob's center along it
/// ([`Layout::knob_x`]); `dy` moves the row to the drawn part (minus the
/// scroll).
fn slider(canvas: &mut Canvas<'_>, theme: &Theme, track: Span, row: Row, at: u32, dy: i64) {
    if track.width == 0 {
        return;
    }
    let y0 = i64::from(row.y0) - dy;
    let y1 = i64::from(row.y1) - dy;
    if y1 <= 0 || y0 >= i64::from(canvas.height()) {
        return;
    }
    let row = Row {
        y0: y0.max(0) as u32,
        y1: y1.max(0) as u32,
    };
    let knob = (row.y1.saturating_sub(row.y0) * 3 / 5).max(2);
    let thick = (knob / 3).max(2);
    let middle = row.y0 + (row.y1.saturating_sub(row.y0)) / 2;
    let y0 = middle.saturating_sub(thick / 2);
    let y1 = y0.saturating_add(thick);
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
