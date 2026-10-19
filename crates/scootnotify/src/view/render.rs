//! Cards to pixels: the stack's repaint.
//!
//! Replays a [`Stack`](super::layout::Stack): fills, borders, text,
//! icons, progress, buttons, the "+N more" pill. No layout, no
//! allocation: every string and rectangle it reads was built at layout
//! time, and the glyph cache is warm after the first paint, which is
//! what the zero-allocation pin measures.

use std::collections::HashMap;

use scootui::paint::{Canvas, Corners, Span};
use scootui::text::Text;

use crate::config::Config;
use crate::look::Look;
use crate::bus::hints::Urgency;
use super::layout::{Card, Rect, Stack};

/// The decoded icon size, in device pixels: the 48 px slot at scale 2.
/// Scale 1 samples it down at draw time (nearest, no allocation).
pub const ICON_SIDE: u32 = 96;

/// A decoded icon: `side × side` premultiplied BGRA, built once at
/// arrival and blitted on every repaint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconBits {
    /// The side, in device pixels ([`ICON_SIDE`]).
    pub side: u32,
    /// The pixels, premultiplied BGRA, `side × side × 4` bytes.
    pub pixels: Vec<u8>,
}

/// What the pointer rests on: the hovered card, and the hovered button
/// within it, if any. The hovered card's expiry pauses; the hovered
/// button draws filled.
pub type Hover = Option<(u32, Option<usize>)>;

/// Paints `stack` into `canvas` (cleared first: the gaps stay
/// transparent and click through). Allocates nothing after warm-up: the
/// lines, labels and rectangles are the layout's, and the glyph cache
/// holds every drawn glyph after the first paint.
pub fn paint(
    canvas: &mut Canvas<'_>,
    text: &mut Text,
    stack: &Stack,
    config: &Config,
    look: &Look,
    urgencies: &HashMap<u32, Urgency>,
    hover: Hover,
    icons: &HashMap<u32, IconBits>,
) {
    canvas.clear();
    let em = look.font_size_px;
    let clip = Span {
        x: 0,
        width: canvas.width(),
    };
    for card in &stack.cards {
        let urgency = urgencies.get(&card.id).copied().unwrap_or(Urgency::Normal);
        paint_card(canvas, text, card, config, look, em, urgency, clip, hover, icons);
    }
    if stack.hidden > 0 {
        paint_pill(canvas, text, stack, look, em, clip);
    }
}

/// Paints one card: the urgency ring, the surface, the icon, the lines,
/// the progress bar, the buttons.
#[allow(clippy::too_many_arguments)]
fn paint_card(
    canvas: &mut Canvas<'_>,
    text: &mut Text,
    card: &Card,
    config: &Config,
    look: &Look,
    em: f32,
    urgency: Urgency,
    clip: Span,
    hover: Hover,
    icons: &HashMap<u32, IconBits>,
) {
    let border = match urgency {
        Urgency::Low => look.border,
        Urgency::Normal => look.focus,
        Urgency::Critical => look.theme.urgent,
    };
    let radius = config.radius;
    let alpha = (look.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    let rect = card.rect;
    if config.border_px > 0 {
        fill_card(canvas, rect, radius, border, 255);
        let inset = config.border_px;
        let inner = Rect {
            x: rect.x + inset,
            y: rect.y + inset,
            width: rect.width.saturating_sub(2 * inset),
            height: rect.height.saturating_sub(2 * inset),
        };
        fill_card(canvas, inner, radius.saturating_sub(inset), look.surface, alpha);
    } else {
        fill_card(canvas, rect, radius, look.surface, alpha);
    }
    if card.has_icon {
        paint_icon(canvas, text, card, look, em, clip, config.border_px, icons);
    }
    for line in &card.summary {
        paint_line(text, canvas, line, look.theme.foreground, look, em, clip);
    }
    paint_line(text, canvas, &card.meta, look.theme.dim, look, em, clip);
    for line in &card.body {
        paint_line(text, canvas, line, look.theme.foreground, look, em, clip);
    }
    if let (Some(value), Some(bar)) = (card.progress, card.progress_bar) {
        paint_progress(canvas, bar, look, value);
    }
    let hovered_button = match hover {
        Some((id, button)) if id == card.id => button,
        _ => None,
    };
    for (index, button) in card.buttons.iter().enumerate() {
        paint_button(text, canvas, button, look, em, clip, hovered_button == Some(index));
    }
}

/// Fills `rect` with `color` at `alpha` (255 opaque) and rounded corners
/// of `radius`: the card fill and its urgency ring. The opaque path is
/// the shared rounded fill; the translucent path blends coverage × alpha
/// per pixel through the shared blender (the gaps stay transparent
/// either way).
fn fill_card(canvas: &mut Canvas<'_>, rect: Rect, radius: u32, color: scootui::color::Color, alpha: u8) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let radius = radius.min(rect.width / 2).min(rect.height / 2);
    if alpha == 255 {
        let corners = Corners::new(radius);
        canvas.fill_rounded(
            rect.x,
            rect.y,
            rect.x + rect.width,
            rect.y + rect.height,
            color,
            &corners,
        );
        return;
    }
    let clip = Span {
        x: 0,
        width: canvas.width(),
    };
    let clip_y = canvas.height();
    for y in rect.y..rect.y + rect.height {
        if y >= clip_y {
            break;
        }
        for x in rect.x..rect.x + rect.width {
            let coverage = rounded_coverage(x, y, rect, radius);
            if coverage == 0 {
                continue;
            }
            let blended = (u32::from(alpha) * u32::from(coverage) + 127) / 255;
            canvas.blend(x as i64, y as i64, blended.min(255) as u8, color, clip);
        }
    }
}

/// The rounded-rectangle coverage of the pixel at `(x, y)` (0 to 255):
/// 255 in the straight runs, the circle's analytic coverage in the
/// corners (a pixel's center within half a pixel of the circle, as the
/// bar's own corner tables read it).
fn rounded_coverage(x: u32, y: u32, rect: Rect, radius: u32) -> u8 {
    if radius == 0 {
        return 255;
    }
    let r = f64::from(radius);
    // Distances to the near corner center, when inside a corner square.
    let cx = if x < rect.x + radius {
        f64::from(rect.x + radius) - (f64::from(x) + 0.5)
    } else if x >= rect.x + rect.width - radius {
        (f64::from(x) + 0.5) - f64::from(rect.x + rect.width - radius)
    } else {
        return 255;
    };
    let cy = if y < rect.y + radius {
        f64::from(rect.y + radius) - (f64::from(y) + 0.5)
    } else if y >= rect.y + rect.height - radius {
        (f64::from(y) + 0.5) - f64::from(rect.y + rect.height - radius)
    } else {
        return 255;
    };
    let distance = (cx * cx + cy * cy).sqrt();
    ((r + 0.5 - distance).clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Paints the icon slot: decoded pixels in the 48 px box, or the app's
/// initial as a letter tile when nothing decoded (a theme name with no
/// file, a refused image: v1 never leaves the slot blank). The tile is a
/// dim plate with the background's letter; image pixels cover its middle
/// (their fit leaves transparent corners over the plate).
fn paint_icon(
    canvas: &mut Canvas<'_>,
    text: &mut Text,
    card: &Card,
    look: &Look,
    em: f32,
    clip: Span,
    border: u32,
    icons: &HashMap<u32, IconBits>,
) {
    let side = super::layout::ICON_PX;
    let ix = card.rect.x + border + super::layout::PAD;
    let iy = card.rect.y + border + super::layout::PAD;
    let corners = Corners::new(8);
    canvas.fill_rounded(ix, iy, ix + side, iy + side, look.theme.dim, &corners);
    let metrics = text.metrics(em);
    // The tile letter on the stack (no format!: the repaint allocates
    // nothing).
    let mut buf = [0u8; 4];
    let tile = card.tile.encode_utf8(&mut buf);
    let tile_w = text.measure(None, tile, em) as i64;
    text.draw(
        canvas,
        None,
        tile,
        em,
        ix as i64 + (side as i64 - tile_w) / 2,
        iy as i64 + (side as i64 + metrics.ascent.ceil() as i64 - metrics.descent.ceil() as i64) / 2,
        look.theme.background,
        clip,
    );
    if let Some(bits) = icons.get(&card.id) {
        blit(canvas, bits, ix, iy, side, clip);
    }
}

/// Blits decoded `bits` into the `side`-logical-pixels box at `(ix,
/// iy)`: nearest-sampled to the box, clipped, premultiplied BGRA over
/// whatever the card fill drew.
fn blit(canvas: &mut Canvas<'_>, bits: &IconBits, ix: u32, iy: u32, side: u32, clip: Span) {
    if bits.side == 0
        || side == 0
        || bits.pixels.len() < bits.side as usize * bits.side as usize * 4
    {
        return;
    }
    let src = bits.side as usize;
    for dy in 0..side {
        let sy = (dy as usize * src) / side as usize;
        for dx in 0..side {
            let sx = (dx as usize * src) / side as usize;
            let at = (sy * src + sx) * 4;
            let Some(pixel) = bits.pixels.get(at..at + 4) else {
                continue;
            };
            canvas.blend_premultiplied(
                ix as i64 + dx as i64,
                iy as i64 + dy as i64,
                [pixel[0], pixel[1], pixel[2], pixel[3]],
                clip,
            );
        }
    }
}

/// Paints one wrapped line in `base` (`accent` for `a` runs): bold runs
/// drawn twice 1 px apart, underlines ruled under the baseline.
fn paint_line(
    text: &mut Text,
    canvas: &mut Canvas<'_>,
    line: &super::layout::Line,
    base: scootui::color::Color,
    look: &Look,
    em: f32,
    clip: Span,
) {
    let mut pen = line.x;
    for run in &line.runs {
        let color = if run.link {
            look.theme.accent
        } else {
            base
        };
        text.draw(canvas, None, &run.text, em, pen, line.baseline, color, clip);
        if run.bold {
            text.draw(canvas, None, &run.text, em, pen + 1, line.baseline, color, clip);
        }
        let width = text.measure(None, &run.text, em) as i64 + i64::from(u8::from(run.bold));
        if run.underline {
            canvas.fill_rect(
                Span {
                    x: pen.max(0) as u32,
                    width: width.max(0) as u32,
                },
                (line.baseline + 2).max(0) as u32,
                (line.baseline + 3).max(0) as u32,
                color,
            );
        }
        pen += width;
    }
}

/// Paints the progress bar into its track: a dim track, the `selection`
/// fill for the first `value` percent.
fn paint_progress(canvas: &mut Canvas<'_>, bar: Rect, look: &Look, value: u8) {
    canvas.fill_rect(
        Span {
            x: bar.x,
            width: bar.width,
        },
        bar.y,
        bar.y + bar.height,
        look.theme.dim,
    );
    let fill = bar.width * u32::from(value.min(100)) / 100;
    if fill > 0 {
        canvas.fill_rect(
            Span {
                x: bar.x,
                width: fill,
            },
            bar.y,
            bar.y + bar.height,
            look.theme.accent,
        );
    }
}

/// Paints one action button: the `focus` outline, filled with `hover`
/// under the pointer, the label centered in foreground (background ink
/// on the hover fill).
fn paint_button(
    text: &mut Text,
    canvas: &mut Canvas<'_>,
    button: &super::layout::Button,
    look: &Look,
    em: f32,
    clip: Span,
    hovered: bool,
) {
    let rect = button.rect;
    if hovered {
        let corners = Corners::new(6);
        canvas.fill_rounded(
            rect.x,
            rect.y,
            rect.x + rect.width,
            rect.y + rect.height,
            look.theme.hover,
            &corners,
        );
    } else {
        // The 1 px outline, four edges.
        canvas.fill_rect(
            Span {
                x: rect.x,
                width: rect.width,
            },
            rect.y,
            rect.y + 1,
            look.focus,
        );
        canvas.fill_rect(
            Span {
                x: rect.x,
                width: rect.width,
            },
            rect.y + rect.height - 1,
            rect.y + rect.height,
            look.focus,
        );
        canvas.fill_rect(
            Span {
                x: rect.x,
                width: 1,
            },
            rect.y,
            rect.y + rect.height,
            look.focus,
        );
        canvas.fill_rect(
            Span {
                x: rect.x + rect.width - 1,
                width: 1,
            },
            rect.y,
            rect.y + rect.height,
            look.focus,
        );
    }
    let ink = if hovered {
        look.theme.background
    } else {
        look.theme.foreground
    };
    let metrics = text.metrics(em);
    let middle = rect.y as i64 + rect.height as i64 / 2;
    let baseline = middle + (metrics.ascent.ceil() as i64 - metrics.descent.ceil() as i64) / 2;
    let label_w = text.measure(None, &button.label, em) as i64;
    let lx = rect.x as i64 + (rect.width as i64 - label_w).max(0) / 2;
    text.draw(canvas, None, &button.label, em, lx, baseline, ink, clip);
}

/// Paints the "+N more" pill: dim text centered under the stack. The
/// label was formatted at layout (the repaint never formats).
fn paint_pill(
    canvas: &mut Canvas<'_>,
    text: &mut Text,
    stack: &Stack,
    look: &Look,
    em: f32,
    clip: Span,
) {
    let label = &stack.pill_label;
    let metrics = text.metrics(em);
    let label_w = text.measure(None, &label, em);
    let pill_w = label_w + 2 * super::layout::PAD;
    let pill_h = (metrics.ascent.ceil() + metrics.descent.ceil()) as u32 + super::layout::PAD;
    let x = stack.width.saturating_sub(pill_w) / 2;
    let y = stack.height.saturating_sub(pill_h);
    canvas.fill_pill(Span { x, width: pill_w }, y, y + pill_h, 8, look.theme.dim);
    let baseline = y as i64 + metrics.ascent.ceil() as i64 + super::layout::PAD as i64 / 2;
    text.draw(
        canvas,
        None,
        &label,
        em,
        x as i64 + super::layout::PAD as i64,
        baseline,
        look.theme.background,
        clip,
    );
}

#[cfg(test)]
mod tests;
