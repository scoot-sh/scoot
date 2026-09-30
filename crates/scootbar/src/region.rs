//! The bar's rounded shape as rectangles, for the surface's input region:
//! the pixels a click may land on. Pure, so the shape is a unit test.
//!
//! The compositor honors `wl_surface.set_input_region` on a layer surface
//! (scoot asks the surface tree, `layer_surface_under` in its `state.rs`,
//! and a declined point falls through to whatever is behind); the
//! headless-scoot test in `tests/appearance.rs` clicks a cut corner and
//! pins it. So the region is the bar's own shape, corners cut, rather than
//! its bounding box: a click in the cut corner reaches the desktop or the
//! window behind, not an invisible bar.
//!
//! Exact, one rectangle per corner row (equal neighbours merged), not the
//! cross of two rectangles the opaque region makes: the cross would leave
//! the visible arc inside each corner square unclickable. At most `2r + 1`
//! rectangles, built when the size or radius changes, never per frame.

#[cfg(test)]
mod tests;

/// A rectangle in surface-local logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// The radius the paint draws for a `width` x `height` bar asked for
/// `radius`: cut back to what the bar can hold (`Style::corners`, in
/// logical pixels).
pub fn effective_radius(radius: u32, width: u32, height: u32) -> u32 {
    radius.min(width / 2).min(height / 2)
}

/// How far in from the left edge row `y` (0 at the top, of `radius` rows)
/// of a corner starts being inside the shape: the first pixel the paint
/// gives any coverage (`Corners::new`: its center within half a pixel of
/// the circle), so no visible pixel is left unclickable.
fn cut(radius: u32, y: u32) -> u32 {
    let r = f64::from(radius);
    // The row's center, from the circle's center.
    let dy = r - (f64::from(y) + 0.5);
    let reach = r + 0.5;
    let dx = (reach * reach - dy * dy).max(0.0).sqrt();
    (r - dx - 0.5).ceil().clamp(0.0, r) as u32
}

/// The rectangles of a `width` x `height` bar with corners of `radius`
/// (already [`effective_radius`]), into `out` (cleared first). A radius of
/// 0 is the one whole rectangle.
pub fn input_rects(width: u32, height: u32, radius: u32, out: &mut Vec<Rect>) {
    out.clear();
    if width == 0 || height == 0 {
        return;
    }
    let radius = effective_radius(radius, width, height);
    // The rows between the corners, full width.
    out.push(Rect {
        x: 0,
        y: radius,
        width,
        height: height - 2 * radius,
    });
    // Each corner row of the top, and its mirror at the bottom, with runs
    // of equal rows merged.
    let mut y = 0;
    while y < radius {
        let inset = cut(radius, y);
        let mut rows = 1;
        while y + rows < radius && cut(radius, y + rows) == inset {
            rows += 1;
        }
        let row = Rect {
            x: inset,
            y,
            width: width - 2 * inset,
            height: rows,
        };
        out.push(row);
        out.push(Rect {
            y: height - y - rows,
            ..row
        });
        y += rows;
    }
}
