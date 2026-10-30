//! The card stack's geometry: cards to rectangles, text to lines.
//!
//! Pure over its inputs (the live notifications, the config, the
//! resolved look size): no Wayland, no filesystem, no clock. Layout runs
//! when the content changes — an arrival, a close, an expiry, a re-theme —
//! and never per frame: the repaint replays the stored lines, which is
//! what the zero-allocation pin covers. Anything time-varying (the age in
//! the meta line) is frozen at layout, so no tick wakes the loop.

use scootui::paint::Pad;
use scootui::text::Text;

use crate::config::Config;
use crate::model::store::Notification;

/// The icon slot, in logical pixels: the design's 48 px, left.
pub const ICON_PX: u32 = 48;
/// The content pad inside the border, in logical pixels.
pub const PAD: u32 = 10;
/// The gap between cards, in logical pixels: the gaps click through
/// (the input region is the cards' union).
pub const GAP: u32 = 8;
/// The progress bar's height, in logical pixels.
pub const PROGRESS_H: u32 = 6;
/// The action row's extra vertical pad, in logical pixels.
pub const BUTTON_PAD_Y: u32 = 6;
/// The action label's horizontal pad, in logical pixels.
pub const BUTTON_PAD_X: u32 = 10;

/// A rectangle in surface-local logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// One styled run inside a line: text that draws in one color.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// The text.
    pub text: String,
    /// `b`: drawn twice, 1 px apart (synthetic emboldening: this stack
    /// has no bold face, and the spec's shaping gate kept rustybuzz out
    /// of v1).
    pub bold: bool,
    /// `u` (and `a`): a 1 px rule under the baseline.
    pub underline: bool,
    /// `a`: accent text, never a link in v1.
    pub link: bool,
}

/// One wrapped line: runs that draw left to right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// The runs.
    pub runs: Vec<Run>,
    /// The pen's left edge, in surface-local logical pixels.
    pub x: i64,
    /// The baseline, in surface-local logical pixels.
    pub baseline: i64,
}

/// One action button: what the pointer hit-tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    /// The action key (`ActionInvoked` carries this).
    pub key: String,
    /// The button's rectangle, in surface-local logical pixels.
    pub rect: Rect,
    /// The drawn label (ellipsized to fit).
    pub label: String,
}

/// One visible card: every rectangle the paint and the pointer need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    /// The notification id.
    pub id: u32,
    /// The card's rectangle, in surface-local logical pixels.
    pub rect: Rect,
    /// The summary, wrapped to at most 2 lines with an ellipsis.
    pub summary: Vec<Line>,
    /// The `app · age` line.
    pub meta: Line,
    /// The body, wrapped to at most `body_lines` with an ellipsis.
    pub body: Vec<Line>,
    /// The body click target (the summary, meta and body rows): a left
    /// click here invokes `default`, or dismisses without one.
    pub body_hit: Rect,
    /// At most 4 buttons (`default` is never a button).
    pub buttons: Vec<Button>,
    /// The progress bar fill, 0 to 100 (`value`).
    pub progress: Option<u8>,
    /// The progress bar's rectangle (the track; the fill is the first
    /// `progress` percent of it).
    pub progress_bar: Option<Rect>,
    /// Whether the card shows the icon slot.
    pub has_icon: bool,
    /// The letter tile's letter: the app name's first alphanumeric,
    /// uppercased, or '•' without one. Drawn when no icon decoded.
    pub tile: char,
}

impl Card {
    /// This card with its origin moved to `(-dx, -dy)`: every rectangle
    /// and line `dx` left and `dy` up, so a card painted into an
    /// off-screen layer starts at the layer's corner. Nothing in a card
    /// lies above or left of `(dx, dy)` when it is the card's own corner
    /// (less its shadow's room); a coordinate that would go negative stops
    /// at zero rather than wrapping. Allocates the clone: runs when a card
    /// starts moving, never per frame.
    pub fn translated(&self, dx: u32, dy: u32) -> Card {
        let mut card = self.clone();
        let back = |rect: &mut Rect| {
            rect.x = rect.x.saturating_sub(dx);
            rect.y = rect.y.saturating_sub(dy);
        };
        let back_line = |line: &mut Line| {
            line.x -= i64::from(dx);
            line.baseline -= i64::from(dy);
        };
        back(&mut card.rect);
        back(&mut card.body_hit);
        card.summary.iter_mut().for_each(back_line);
        back_line(&mut card.meta);
        card.body.iter_mut().for_each(back_line);
        if let Some(bar) = card.progress_bar.as_mut() {
            back(bar);
        }
        for button in &mut card.buttons {
            back(&mut button.rect);
        }
        card
    }
}

/// The laid-out stack: the surface size, the visible cards newest on
/// top, the hidden count, and the input region (the cards' union, so
/// the gaps click through).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stack {
    /// The surface width, in logical pixels, [`pad`](Stack::pad) included.
    pub width: u32,
    /// The surface height, in logical pixels, [`pad`](Stack::pad) included.
    pub height: u32,
    /// The room around the cards a shadow paints in, which every
    /// rectangle below already accounts for. Zero without a shadow:
    /// the surface is then exactly the cards.
    pub pad: Pad,
    /// The visible cards, newest first.
    pub cards: Vec<Card>,
    /// Cards past `max_visible`, covered by the "+N more" pill.
    pub hidden: usize,
    /// The pill's label ("+N more"), built at layout (the repaint never
    /// formats).
    pub pill_label: String,
    /// The input region: one rectangle per card (the union, not the
    /// bounding box).
    pub input: Vec<Rect>,
}

impl Stack {
    /// A stack with no cards: what an empty store lays out as (before
    /// any padding).
    #[cfg(test)]
    pub fn default_empty() -> Stack {
        Stack {
            width: 0,
            height: 0,
            pad: Pad::ZERO,
            cards: Vec::new(),
            hidden: 0,
            pill_label: String::new(),
            input: Vec::new(),
        }
    }
}

/// Lays out `live` (oldest first) newest-on-top: at most
/// `config.max_visible` cards, the rest counted hidden. `now_ms` freezes
/// the meta ages (CLOCK_REALTIME ms, the store's `created_ms` clock).
/// `em` is the look's text size. Allocates (Strings, Vecs): this runs on
/// content change, never per repaint.
pub fn layout(live: &[&Notification], config: &Config, text: &Text, em: f32, now_ms: u64) -> Stack {
    let metrics = text.metrics(em);
    let line_h = (metrics.ascent + metrics.descent + 4.0).ceil().max(1.0) as u32;
    let ascent = metrics.ascent.ceil().max(1.0) as u32;
    let width = config.width_px.max(10);
    let border = config.border_px;
    let content_x = border + PAD;
    let content_w = width.saturating_sub(2 * content_x);
    let max_visible = config.max_visible.max(1) as usize;
    let body_lines = config.body_lines.max(1) as usize;

    let mut cards = Vec::new();
    let mut y = 0u32;
    for note in live.iter().rev().take(max_visible) {
        let spans = crate::markup::parse(&note.body);
        let icon_w = if has_icon_slot(note) {
            ICON_PX + PAD
        } else {
            0
        };
        let text_w = content_w.saturating_sub(icon_w);
        let text_x = content_x + icon_w;
        let mut card_y = y + border + PAD;
        let summary = wrap_spans(
            &summary_runs(&note.summary),
            text,
            em,
            text_w,
            2,
            text_x,
            card_y,
            ascent,
            line_h,
        );
        card_y += summary.len() as u32 * line_h;
        let meta_text = meta_text(note, now_ms);
        let meta = Line {
            runs: vec![Run {
                text: meta_text,
                bold: false,
                underline: false,
                link: false,
            }],
            x: i64::from(text_x),
            baseline: i64::from(card_y + ascent),
        };
        card_y += line_h;
        let body = wrap_spans(
            &spans, text, em, text_w, body_lines, text_x, card_y, ascent, line_h,
        );
        card_y += body.len() as u32 * line_h;
        let progress_bar = note.value.map(|_| {
            let bar = Rect {
                x: content_x,
                y: card_y,
                width: content_w,
                height: PROGRESS_H,
            };
            card_y += PROGRESS_H + PAD;
            bar
        });
        let mut buttons = Vec::new();
        let shown: Vec<_> = note
            .actions
            .iter()
            .filter(|pair| pair.key != "default")
            .take(4)
            .collect();
        if !shown.is_empty() {
            let per = content_w / shown.len() as u32;
            let mut bx = content_x;
            for pair in shown {
                let avail = per.saturating_sub(2 * BUTTON_PAD_X).max(1);
                let label = ellipsize(&pair.label, text, em, avail);
                let bw = if bx + per > content_x + content_w {
                    content_x + content_w - bx
                } else {
                    per
                };
                buttons.push(Button {
                    key: pair.key.clone(),
                    rect: Rect {
                        x: bx,
                        y: card_y,
                        width: bw.max(1),
                        height: line_h + 2 * BUTTON_PAD_Y,
                    },
                    label,
                });
                bx += per;
            }
            card_y += line_h + 2 * BUTTON_PAD_Y;
        }
        let text_h = card_y - (y + border + PAD);
        let icon_h = if has_icon_slot(note) { ICON_PX } else { 0 };
        let h = border + PAD + text_h.max(icon_h) + PAD + border;
        let rect = Rect {
            x: 0,
            y,
            width,
            height: h,
        };
        let body_hit = Rect {
            x: content_x,
            y: y + border + PAD,
            width: content_w,
            height: h.saturating_sub(2 * (border + PAD)),
        };
        cards.push(Card {
            id: note.id,
            rect,
            summary,
            meta,
            body,
            body_hit,
            buttons,
            progress: note.value,
            progress_bar,
            has_icon: has_icon_slot(note),
            tile: tile_letter(&note.app_name),
        });
        y += h + GAP;
    }
    let hidden = live.len().saturating_sub(cards.len());
    let pill_label = if hidden == 0 {
        String::new()
    } else {
        format!("+{hidden} more")
    };
    let height = if cards.is_empty() {
        0
    } else {
        y - GAP + pill_h(hidden, line_h)
    };
    let input = cards.iter().map(|card| card.rect).collect();
    Stack {
        width,
        height,
        pad: Pad::ZERO,
        cards,
        hidden,
        pill_label,
        input,
    }
}

/// Grows `stack`'s surface by `pad` on each side and moves everything in
/// it (cards, text, buttons, progress bars, the input region) to match,
/// so a shadow has room to paint around the cards. The input region stays
/// the cards' rectangles: the shadow band clicks through. An empty stack
/// stays empty (it hides, rather than showing a surface of pure padding).
/// `stack` is `layout`'s own output, not yet padded.
pub fn inset(mut stack: Stack, pad: Pad) -> Stack {
    if pad.is_zero() || stack.cards.is_empty() {
        return stack;
    }
    debug_assert!(stack.pad.is_zero(), "a stack is inset once");
    let (dx, dy) = (pad.left, pad.top);
    let shift = |rect: &mut Rect| {
        rect.x = rect.x.saturating_add(dx);
        rect.y = rect.y.saturating_add(dy);
    };
    let shift_line = |line: &mut Line| {
        line.x += i64::from(dx);
        line.baseline += i64::from(dy);
    };
    for card in &mut stack.cards {
        shift(&mut card.rect);
        shift(&mut card.body_hit);
        card.summary.iter_mut().for_each(shift_line);
        shift_line(&mut card.meta);
        card.body.iter_mut().for_each(shift_line);
        if let Some(bar) = card.progress_bar.as_mut() {
            shift(bar);
        }
        for button in &mut card.buttons {
            shift(&mut button.rect);
        }
    }
    stack.input.iter_mut().for_each(shift);
    stack.width = stack
        .width
        .saturating_add(pad.left)
        .saturating_add(pad.right);
    stack.height = stack
        .height
        .saturating_add(pad.top)
        .saturating_add(pad.bottom);
    stack.pad = pad;
    stack
}

/// The "+N more" pill's height: one padded line, or nothing.
fn pill_h(hidden: usize, line_h: u32) -> u32 {
    if hidden == 0 { 0 } else { line_h + PAD }
}

/// The letter tile's letter: the app name's first alphanumeric,
/// uppercased, or '•' without one.
fn tile_letter(app: &str) -> char {
    app.chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().next().unwrap_or('•'))
        .unwrap_or('•')
}

/// Whether the card shows the icon slot: any icon but none.
fn has_icon_slot(note: &Notification) -> bool {
    !matches!(note.icon, crate::model::store::Icon::None)
}

/// The summary as one bold run.
fn summary_runs(summary: &str) -> Vec<crate::markup::Span> {
    vec![crate::markup::Span {
        text: summary.to_owned(),
        bold: true,
        italic: false,
        underline: false,
    }]
}

/// The meta line: `app · age`, the age frozen at layout.
fn meta_text(note: &Notification, now_ms: u64) -> String {
    let age = age_text(now_ms.saturating_sub(note.created_ms));
    if note.app_name.is_empty() {
        age.to_owned()
    } else {
        format!("{} · {age}", note.app_name)
    }
}

/// A frozen age: under 10 s is "now", then seconds, minutes, hours,
/// days. Never ticks (no timer owns it).
fn age_text(elapsed_ms: u64) -> String {
    let secs = elapsed_ms / 1000;
    if secs < 10 {
        String::from("now")
    } else if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

/// Wraps markup spans into at most `max_lines` lines of styled runs.
#[allow(clippy::too_many_arguments)] // A layout helper with one job.
fn wrap_spans(
    spans: &[crate::markup::Span],
    text: &Text,
    em: f32,
    max_width: u32,
    max_lines: usize,
    x: u32,
    y: u32,
    ascent: u32,
    line_h: u32,
) -> Vec<Line> {
    // Flatten to characters with their style: the wrap owns this (layout
    // time, never the repaint).
    let mut chars: Vec<(char, bool, bool, bool)> = Vec::new();
    for span in spans {
        // `img` alt text already reads as text; control characters other
        // than tab never reach the screen.
        for c in span.text.chars().filter(|c| !c.is_control() || *c == '\t') {
            chars.push((
                if c == '\t' { ' ' } else { c },
                span.bold,
                span.underline,
                span_is_link(span),
            ));
        }
    }
    // Hard newlines in the body break lines (the spec's bodies carry
    // them); `\n` never draws.
    let mut lines: Vec<Line> = Vec::new();
    let mut at = 0usize;
    while at < chars.len() && lines.len() < max_lines {
        let last = lines.len() + 1 == max_lines;
        let (next, truncated) = take_line(&chars, at, text, em, max_width);
        let mut runs: Vec<Run> = Vec::new();
        for (c, bold, underline, link) in &chars[at..next] {
            if *c == '\n' {
                continue;
            }
            if let Some(run) = runs.last_mut()
                && run.bold == *bold
                && run.underline == *underline
                && run.link == *link
            {
                run.text.push(*c);
                continue;
            }
            runs.push(Run {
                text: String::from(*c),
                bold: *bold,
                underline: *underline,
                link: *link,
            });
        }
        if truncated && last {
            ellipsize_runs(&mut runs, text, em, max_width);
        }
        // An empty source still draws its line (a blank row keeps the
        // card's shape); an empty wrap of nothing draws nothing.
        if runs.is_empty() && next == at {
            break;
        }
        lines.push(Line {
            runs,
            x: i64::from(x),
            baseline: i64::from(y + lines.len() as u32 * line_h + ascent),
        });
        at = if next == at { at + 1 } else { next };
        // Skip the newline that ended the line, and leading spaces of
        // the next (a wrapped line never starts with a space).
        if chars.get(at).is_some_and(|(c, _, _, _)| *c == '\n') {
            at += 1;
        }
        while lines.len() < max_lines
            && chars.get(at).is_some_and(|(c, _, _, _)| *c == ' ')
            && chars[..at]
                .iter()
                .any(|(c, _, _, _)| *c != ' ' && *c != '\n')
        {
            at += 1;
        }
    }
    // Trailing newline: the ellipsis already covers the overflow; a body
    // of only newlines draws one blank line.
    if lines.is_empty() && !chars.is_empty() {
        lines.push(Line {
            runs: Vec::new(),
            x: i64::from(x),
            baseline: i64::from(y + ascent),
        });
    }
    lines
}

/// Takes one line from `chars` at `at`: the end index and whether text
/// remains past it. Breaks at spaces and after CJK characters;
/// hard-breaks a word wider than the line.
fn take_line(
    chars: &[(char, bool, bool, bool)],
    at: usize,
    text: &Text,
    em: f32,
    max_width: u32,
) -> (usize, bool) {
    let mut pen = 0.0f32;
    let mut break_at: Option<usize> = None;
    let mut i = at;
    while i < chars.len() {
        let (c, _, _, _) = chars[i];
        if c == '\n' {
            return (i, i + 1 < chars.len());
        }
        pen += text.advance(c, em);
        if pen.round() > max_width as f32 {
            // Over the edge: break at the last opportunity, else here
            // (a hard break: an overlong word still shows).
            if let Some(end) = break_at {
                return (end, true);
            }
            return (i.max(at + 1), true);
        }
        if c == ' ' {
            break_at = Some(i);
        } else if is_cjk(c) {
            break_at = Some(i + 1);
        }
        i += 1;
    }
    (i, false)
}

/// Whether `c` breaks after itself: CJK Unified, Hiragana, Katakana,
/// Hangul (the line breaking v1 does without a table crate).
fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{2e80}'..='\u{9fff}'
        | '\u{ac00}'..='\u{d7af}'
        | '\u{f900}'..='\u{faff}'
        | '\u{ff00}'..='\u{ffef}')
}

/// Ellipsizes the last run to fit `max_width`: appends "…" to the
/// longest fitting prefix.
fn ellipsize_runs(runs: &mut Vec<Run>, text: &Text, em: f32, max_width: u32) {
    let ellipsis_w = text.measure(None, "…", em);
    if ellipsis_w > max_width {
        runs.clear();
        return;
    }
    let budget = max_width - ellipsis_w;
    let mut pen = 0.0f32;
    let mut end_run = runs.len();
    let mut end_char = 0usize;
    'outer: for (r, run) in runs.iter().enumerate() {
        for (k, c) in run.text.chars().enumerate() {
            pen += text.advance(c, em);
            if pen.round() > budget as f32 {
                end_run = r;
                end_char = k;
                break 'outer;
            }
        }
        end_char = run.text.chars().count();
        end_run = r;
    }
    runs.truncate(end_run + 1);
    if let Some(run) = runs.last_mut() {
        let kept: String = run.text.chars().take(end_char).collect();
        run.text = format!("{kept}…");
    }
}

/// Whether the span draws as accent text: `a` only (v1 draws links as
/// underlined accent, never clickable).
fn span_is_link(span: &crate::markup::Span) -> bool {
    span.underline && is_anchor_text(&span.text)
}

/// `a` tags parse with `underline` set; plain `u` text is not a link.
/// The parser does not keep the tag, so v1 treats every underline as
/// accent text and none as a link target: this returns false, and the
/// paint reads `underline` for the rule and `link` never. Kept as the
/// one place that changes when links become clickable.
fn is_anchor_text(_text: &str) -> bool {
    false
}

/// Ellipsizes `full` to `available` device pixels: the longest fitting
/// prefix with "…", or the empty string when even that does not fit.
pub fn ellipsize(full: &str, text: &Text, em: f32, available: u32) -> String {
    if text.measure(None, full, em) <= available {
        return full.to_owned();
    }
    let ellipsis_w = text.measure(None, "…", em);
    if ellipsis_w > available {
        return String::new();
    }
    let budget = available - ellipsis_w;
    let mut pen = 0.0f32;
    let mut len = 0usize;
    for c in full.chars() {
        pen += text.advance(c, em);
        if pen.round() > budget as f32 {
            break;
        }
        len += c.len_utf8();
    }
    format!("{}…", &full[..len])
}

#[cfg(test)]
mod tests;
