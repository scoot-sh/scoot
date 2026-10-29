//! A filled rounded rectangle for the selection highlight and the letter
//! tile. Allocation-free (the corner coverage is computed per pixel, a
//! few hundred pixels at most) and clipped by the canvas.

use crate::layout::Rect;
use scootui::color::Color;
use scootui::paint::{Canvas, Span};

/// `color` over `rect` with corners of `radius` pixels (cut back to what
/// the rectangle holds). Straight runs are written whole; only the
/// corner pixels blend, so the fill composites over what is there.
pub fn fill_round_rect(canvas: &mut Canvas<'_>, rect: Rect, radius: u32, color: Color) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let radius = radius.min(rect.width / 2).min(rect.height / 2);
    let full = Span {
        x: 0,
        width: canvas.width(),
    };
    for dy in 0..rect.height {
        let y = rect.y.saturating_add(dy);
        let from_edge = dy.min(rect.height - 1 - dy);
        if from_edge >= radius {
            canvas.fill_rect(
                Span {
                    x: rect.x,
                    width: rect.width,
                },
                y,
                y.saturating_add(1),
                color,
            );
            continue;
        }
        canvas.fill_rect(
            Span {
                x: rect.x.saturating_add(radius),
                width: rect.width - 2 * radius,
            },
            y,
            y.saturating_add(1),
            color,
        );
        let r = radius as f32;
        for lx in 0..radius {
            let reach = (r - (lx as f32 + 0.5)).hypot(r - (from_edge as f32 + 0.5));
            let coverage = ((r - reach + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
            for x in [
                rect.x.saturating_add(lx),
                rect.right().saturating_sub(1 + lx),
            ] {
                canvas.blend(i64::from(x), i64::from(y), coverage, color, full);
            }
        }
    }
}

#[cfg(test)]
mod tests;
