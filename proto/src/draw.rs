//! One draw of the launcher panel, shared by the live client and the
//! headless `--bench` timing. scootbar's own `Canvas` + `Text`, nothing else.

use crate::index::Entry;
use crate::{color::Color, paint::{Canvas, Span}, text::Text};

pub const W: u32 = 640;
pub const QUERY_H: u32 = 36;
pub const ROW_H: u32 = 30;
pub const VISIBLE: usize = 10;
pub const PAD: u32 = 4;
pub const H: u32 = QUERY_H + VISIBLE as u32 * ROW_H + PAD * 2;
pub const EM: f32 = 13.0;

pub fn bg() -> Color {
    Color::parse("#1e1e2e").unwrap()
}
pub fn sel() -> Color {
    Color::parse("#2f5b9f").unwrap()
}
pub fn ink() -> Color {
    Color::parse("#e6e6e6").unwrap()
}
pub fn dim() -> Color {
    Color::parse("#8a8a8a").unwrap()
}

pub fn draw_panel(
    pixels: &mut [u8],
    text: &mut Text,
    query: &str,
    order: &[usize],
    entries: &[Entry],
    selected: usize,
) {
    let Some(mut canvas) = Canvas::new(pixels, W, H) else {
        return;
    };
    let full = Span { x: 0, width: W };
    canvas.fill_rect(full, 0, H, bg());
    // Query line.
    let metrics = text.metrics(EM);
    let shown = if query.is_empty() { "…" } else { query };
    let color = if query.is_empty() { dim() } else { ink() };
    let baseline = metrics.baseline(QUERY_H);
    text.draw(&mut canvas, None, shown, EM, 12, baseline.min(QUERY_H as i64 - 4), color, full);
    // Rows.
    for (row, slot) in order.iter().take(VISIBLE).enumerate() {
        let y0 = QUERY_H + PAD + row as u32 * ROW_H;
        let e = &entries[*slot];
        if row == selected {
            canvas.fill_rect(full, y0, y0 + ROW_H, sel());
        }
        let b = metrics.baseline(ROW_H) + y0 as i64;
        let label = if e.generic.is_empty() {
            e.name.clone()
        } else {
            format!("{} — {}", e.name, e.generic)
        };
        text.draw(&mut canvas, None, &label, EM, 12, b.min((y0 + ROW_H - 4) as i64), ink(), full);
    }
    if order.is_empty() {
        let b = metrics.baseline(ROW_H) + (QUERY_H + PAD) as i64;
        text.draw(&mut canvas, None, "No applications", EM, 12, b, dim(), full);
    }
}
