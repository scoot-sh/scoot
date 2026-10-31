//! Cards to pixels: the stack's repaint.
//!
//! Replays a [`Stack`](super::layout::Stack): fills, borders, text,
//! icons, progress, buttons, the "+N more" pill. No layout, no
//! allocation: every string and rectangle it reads was built at layout
//! time, and the glyph cache is warm after the first paint, which is
//! what the zero-allocation pin measures.

use std::collections::HashMap;
use std::time::Instant;

use scootui::color::Color;
use scootui::paint::{Canvas, Shadow, Span};
use scootui::text::Text;

use super::layout::{Card, Rect, Stack};
use super::motion::Motion;
use crate::bus::hints::Urgency;
use crate::config::Config;
use crate::look::style::{DOT_PX, STRIPE_PX};
use crate::look::{CardMark, Look, ShadowStyle};

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
/// lines, labels and rectangles are the layout's, the glyph cache holds
/// every drawn glyph after the first paint, and `shadow`'s tables were
/// built when the stack changed. `shadow` is the look's shadow (`None`
/// for `shadow = "none"`, which paints and costs nothing); every shadow
/// is painted before any card so a card's shadow never lands on its
/// neighbor.
#[allow(clippy::too_many_arguments)] // A paint helper, as the bar's own are.
pub fn paint(
    canvas: &mut Canvas<'_>,
    text: &mut Text,
    stack: &Stack,
    config: &Config,
    look: &Look,
    urgencies: &HashMap<u32, Urgency>,
    hover: Hover,
    icons: &HashMap<u32, IconBits>,
    shadow: Option<&Shadow>,
) {
    canvas.clear();
    paint_stack(
        canvas,
        text,
        stack,
        config,
        look,
        urgencies,
        hover,
        icons,
        shadow,
        |_| false,
    );
}

/// Paints `stack` while some cards are in motion: the cards at rest
/// through the same path as [`paint`] (so a stack between animations is
/// byte for byte the static one), the leaving and moving cards composited
/// from their layers at `now`. A bottom-anchored surface held taller than
/// its layout paints the static cards that much lower, where they sit.
#[allow(clippy::too_many_arguments)] // A paint helper, as the bar's own are.
pub fn paint_moving(
    canvas: &mut Canvas<'_>,
    text: &mut Text,
    stack: &Stack,
    config: &Config,
    look: &Look,
    urgencies: &HashMap<u32, Urgency>,
    hover: Hover,
    icons: &HashMap<u32, IconBits>,
    shadow: Option<&Shadow>,
    motion: &Motion,
    now: Instant,
) {
    canvas.clear();
    let shift = motion.shift(canvas.height(), stack.height);
    {
        let mut base = canvas.below(shift);
        paint_stack(
            &mut base,
            text,
            stack,
            config,
            look,
            urgencies,
            hover,
            icons,
            shadow,
            |id| motion.moving(id),
        );
    }
    motion.compose(canvas, now);
}

/// Paints one card into its own layer: `card` is at the layer's origin
/// ([`Card::translated`](super::layout::Card::translated)), its shadow
/// inside the layer's margin. `button` is the hovered button, if any. The
/// layer was cleared by the caller.
#[allow(clippy::too_many_arguments)] // A paint helper, as the bar's own are.
pub fn paint_layer(
    canvas: &mut Canvas<'_>,
    text: &mut Text,
    card: &Card,
    config: &Config,
    look: &Look,
    urgency: Urgency,
    button: Option<usize>,
    icons: &HashMap<u32, IconBits>,
    shadow: Option<&Shadow>,
) {
    let clip = Span {
        x: 0,
        width: canvas.width(),
    };
    if let Some(shadow) = shadow {
        shadow_of(canvas, shadow, card, config, look, urgency);
    }
    let hover = button.map(|button| (card.id, Some(button)));
    paint_card(
        canvas,
        text,
        card,
        config,
        look,
        look.font_size_px,
        urgency,
        clip,
        hover,
        icons,
    );
}

/// The shadow under one card: black, or the card's ring color for a glow.
fn shadow_of(
    canvas: &mut Canvas<'_>,
    shadow: &Shadow,
    card: &Card,
    config: &Config,
    look: &Look,
    urgency: Urgency,
) {
    let tint = match look.shadow {
        // A glow is the card's own ring color, so a critical card
        // glows in the urgent color.
        ShadowStyle::Glow => ring_color(look, urgency),
        _ => Color { r: 0, g: 0, b: 0 },
    };
    shadow.paint(
        canvas,
        region(card.rect),
        look.card_radius(config.radius),
        tint,
    );
}

/// [`paint`] without the clear, skipping the cards `skip` names (the ones
/// drawn through layers). Every shadow first, then every card.
#[allow(clippy::too_many_arguments)] // A paint helper, as the bar's own are.
fn paint_stack(
    canvas: &mut Canvas<'_>,
    text: &mut Text,
    stack: &Stack,
    config: &Config,
    look: &Look,
    urgencies: &HashMap<u32, Urgency>,
    hover: Hover,
    icons: &HashMap<u32, IconBits>,
    shadow: Option<&Shadow>,
    skip: impl Fn(u32) -> bool,
) {
    let em = look.font_size_px;
    let clip = Span {
        x: 0,
        width: canvas.width(),
    };
    if let Some(shadow) = shadow {
        for card in stack.cards.iter().filter(|card| !skip(card.id)) {
            let urgency = urgencies.get(&card.id).copied().unwrap_or(Urgency::Normal);
            shadow_of(canvas, shadow, card, config, look, urgency);
        }
    }
    for card in stack.cards.iter().filter(|card| !skip(card.id)) {
        let urgency = urgencies.get(&card.id).copied().unwrap_or(Urgency::Normal);
        paint_card(
            canvas, text, card, config, look, em, urgency, clip, hover, icons,
        );
    }
    if stack.hidden > 0 {
        paint_pill(canvas, text, stack, look, em, clip);
    }
}

/// Paints one card: the urgency ring, the surface, the icon, the lines,
/// the progress bar, the buttons.
#[allow(clippy::too_many_arguments)] // A paint helper, as the bar's own are.
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
    let border = ring_color(look, urgency);
    let radius = look.card_radius(config.radius);
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
        fill_gradient(
            canvas,
            inner,
            radius.saturating_sub(inset),
            look.fill_stops(),
            alpha,
        );
    } else {
        fill_gradient(canvas, rect, radius, look.fill_stops(), alpha);
    }
    paint_mark(canvas, card, look, radius, config.border_px);
    if card.has_icon {
        paint_icon(canvas, text, card, look, em, clip, config.border_px, icons);
    }
    let title = Ink {
        strikes: look.title_strikes(),
        tracking: look.title_tracking(),
    };
    for line in &card.summary {
        paint_line(
            text,
            canvas,
            line,
            look.theme.foreground,
            look,
            em,
            clip,
            title,
        );
    }
    paint_line(
        text,
        canvas,
        &card.meta,
        look.theme.dim,
        look,
        em,
        clip,
        BODY_INK,
    );
    for line in &card.body {
        paint_line(
            text,
            canvas,
            line,
            look.theme.foreground,
            look,
            em,
            clip,
            BODY_INK,
        );
    }
    if let (Some(value), Some(bar)) = (card.progress, card.progress_bar) {
        paint_progress(canvas, bar, look, value);
    }
    let hovered_button = match hover {
        Some((id, button)) if id == card.id => button,
        _ => None,
    };
    for (index, button) in card.buttons.iter().enumerate() {
        paint_button(
            text,
            canvas,
            button,
            look,
            em,
            clip,
            hovered_button == Some(index),
        );
    }
}

/// How a line is struck and spaced: a bold run is drawn `strikes` times
/// 1 px apart, and every character steps `tracking` extra pixels.
#[derive(Debug, Clone, Copy)]
struct Ink {
    strikes: u8,
    tracking: f32,
}

/// Body, meta and the bold runs inside them: two strikes, no spacing, as
/// bold has always drawn.
const BODY_INK: Ink = Ink {
    strikes: 2,
    tracking: 0.0,
};

/// The accent a card's mark is drawn in: the app's own color when the look
/// asks for one, else the look's `accent`.
fn mark_color(card: &Card, look: &Look) -> Color {
    card.accent.unwrap_or(look.theme.accent)
}

/// Paints the stripe or the dot (the badge is the icon slot's tile): after
/// the surface and inside it, so the stripe is cut by the card's own
/// rounded (or pill) shape, and before the text. In the card's own
/// coordinates, so a card painted into a motion layer carries its mark.
fn paint_mark(canvas: &mut Canvas<'_>, card: &Card, look: &Look, radius: u32, border: u32) {
    match look.mark {
        CardMark::Stripe => {
            let rect = card.rect;
            let inner = Rect {
                x: rect.x + border,
                y: rect.y + border,
                width: rect.width.saturating_sub(2 * border),
                height: rect.height.saturating_sub(2 * border),
            };
            canvas.fill_rounded_edge(
                region(inner),
                radius.saturating_sub(border),
                STRIPE_PX,
                mark_color(card, look),
            );
        }
        CardMark::Dot => {
            if let Some(dot) = card.dot {
                fill_card(canvas, dot, DOT_PX / 2, mark_color(card, look), 255);
            }
        }
        CardMark::None | CardMark::Badge => {}
    }
}

/// The card's ring color for `urgency`: also the tint of its glow.
fn ring_color(look: &Look, urgency: Urgency) -> Color {
    match urgency {
        Urgency::Low => look.border,
        Urgency::Normal => look.focus,
        Urgency::Critical => look.theme.urgent,
    }
}

/// Fills `rect` with `color` at `alpha` (255 opaque) and rounded corners
/// of `radius`: the urgency ring, the icon tile, the hovered button.
fn fill_card(canvas: &mut Canvas<'_>, rect: Rect, radius: u32, color: Color, alpha: u8) {
    fill_gradient(canvas, rect, radius, (color, color), alpha);
}

/// Fills `rect` like [`fill_card`] with a vertical gradient from the
/// first stop (top) to the second (bottom): the card surface. Equal stops
/// are the solid fill, byte for byte. The shared blender reads correct
/// over any destination (the inset fill over its ring, a translucent
/// fill over its shadow, cartoon text over the fill).
fn fill_gradient(
    canvas: &mut Canvas<'_>,
    rect: Rect,
    radius: u32,
    stops: (Color, Color),
    alpha: u8,
) {
    canvas.fill_rounded_gradient(region(rect), radius, stops.0, stops.1, alpha);
}

/// A layout rectangle as the shared canvas's.
fn region(rect: Rect) -> scootui::region::Rect {
    scootui::region::Rect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

/// Paints the icon slot: decoded pixels in the 48 px box, or the app's
/// initial as a letter tile when nothing decoded (a theme name with no
/// file, a refused image: v1 never leaves the slot blank). The tile is a
/// dim plate with the background's letter; image pixels cover its middle
/// (their fit leaves transparent corners over the plate).
#[allow(clippy::too_many_arguments)] // A paint helper, as the bar's own are.
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
    let ix = card.rect.x + card.pad_x;
    let iy = card.rect.y + border + super::layout::PAD;
    // The plate and the letter's ink: dim with the background's letter, or
    // under a badge the mark's tile with the ink the app accent was
    // darkened (or lightened) to read.
    let (plate, ink) = match (look.mark, card.accent) {
        (CardMark::Badge, Some(accent)) => (accent, look.theme.foreground),
        (CardMark::Badge, None) => (look.theme.accent, look.theme.background),
        _ => (look.theme.dim, look.theme.background),
    };
    fill_card(
        canvas,
        crate::view::layout::Rect {
            x: ix,
            y: iy,
            width: side,
            height: side,
        },
        8,
        plate,
        255,
    );
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
        iy as i64
            + (side as i64 + metrics.ascent.ceil() as i64 - metrics.descent.ceil() as i64) / 2,
        ink,
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

/// Paints one wrapped line in `base` (`accent` for `a` runs): bold runs in
/// the bold face if the text has one (struck once more for each strike past
/// two), else struck `ink.strikes` times 1 px apart; underlines ruled under
/// the baseline, every character stepped `ink.tracking` extra.
#[allow(clippy::too_many_arguments)] // A paint helper, as the bar's own are.
fn paint_line(
    text: &mut Text,
    canvas: &mut Canvas<'_>,
    line: &super::layout::Line,
    base: Color,
    look: &Look,
    em: f32,
    clip: Span,
    ink: Ink,
) {
    let mut pen = line.x;
    for run in &line.runs {
        let color = if run.link { look.theme.accent } else { base };
        let strikes = match (run.bold, text.has_bold()) {
            (false, _) => 1,
            (true, false) => ink.strikes.max(1),
            // The face is the first strike of a two-strike bold.
            (true, true) => ink.strikes.saturating_sub(1).max(1),
        };
        let face = super::layout::face(run.bold);
        for strike in 0..strikes {
            text.draw_tracked(
                canvas,
                None,
                &run.text,
                (em, face, ink.tracking),
                pen + i64::from(strike),
                line.baseline,
                color,
                clip,
            );
        }
        let extra = Text::tracking_extra(&run.text, ink.tracking).round() as i64;
        let width = text.measure_face(None, &run.text, em, face) as i64
            + extra
            + i64::from(strikes.saturating_sub(1));
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
        fill_card(canvas, rect, 6, look.theme.hover, 255);
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
    let label: &str = &stack.pill_label;
    let metrics = text.metrics(em);
    let label_w = text.measure(None, label, em);
    let pill_w = label_w + 2 * super::layout::PAD;
    let pill_h = (metrics.ascent.ceil() + metrics.descent.ceil()) as u32 + super::layout::PAD;
    // Centered under the cards and sitting on their bottom edge, not on
    // the shadow's room below them.
    let cards_w = stack.width.saturating_sub(stack.pad.left + stack.pad.right);
    let x = stack.pad.left + cards_w.saturating_sub(pill_w) / 2;
    let y = stack
        .height
        .saturating_sub(stack.pad.bottom)
        .saturating_sub(pill_h);
    canvas.fill_pill(Span { x, width: pill_w }, y, y + pill_h, 8, look.theme.dim);
    let baseline = y as i64 + metrics.ascent.ceil() as i64 + super::layout::PAD as i64 / 2;
    text.draw(
        canvas,
        None,
        label,
        em,
        x as i64 + super::layout::PAD as i64,
        baseline,
        look.theme.background,
        clip,
    );
}

#[cfg(test)]
mod bold_tests;
#[cfg(test)]
mod depth_tests;
#[cfg(test)]
mod motion_tests;
#[cfg(test)]
mod style_tests;
#[cfg(test)]
pub(crate) mod tests;
