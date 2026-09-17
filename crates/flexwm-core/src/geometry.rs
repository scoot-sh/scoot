//! Integer geometry in logical pixels.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Size {
    pub w: i32,
    pub h: i32,
}

/// An axis-aligned rectangle; `x`/`y` is the top-left corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

impl Size {
    pub const fn new(w: i32, h: i32) -> Self {
        Self { w, h }
    }
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn size(&self) -> Size {
        Size::new(self.w, self.h)
    }

    /// The first column past the rect's right edge. Saturates rather than
    /// overflowing: a rect running off the end of the coordinate space ends
    /// at `i32::MAX`, which is what keeps the on-screen test and
    /// [`Rect::contains`] total over every input.
    pub const fn right(&self) -> i32 {
        self.x.saturating_add(self.w)
    }

    /// The first row past the rect's bottom edge. Saturates, like
    /// [`Rect::right`].
    pub const fn bottom(&self) -> i32 {
        self.y.saturating_add(self.h)
    }

    pub const fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    /// Shrinks every side by `by`, never below zero size. Saturates rather
    /// than overflowing, the same discipline as [`Rect::right`]: an origin
    /// past `i32::MAX` clamps to it, and a margin wider than the rect --
    /// whose doubled width alone can overflow `i32` -- leaves an empty rect
    /// there. A negative `by` still grows the rect.
    pub fn inset(&self, by: i32) -> Rect {
        Rect::new(
            self.x.saturating_add(by),
            self.y.saturating_add(by),
            self.w.saturating_sub(by.saturating_mul(2)).max(0),
            self.h.saturating_sub(by.saturating_mul(2)).max(0),
        )
    }

    /// The overlap of two rectangles, empty (zero width and/or height) when
    /// they don't overlap or either one is itself empty.
    ///
    /// Deliberately not written in terms of [`Rect::right`]/[`Rect::bottom`],
    /// whose `x + w` saturates at `i32::MAX`: both operands here can come
    /// from a client (a layer-shell surface's exclusive zone and margins are
    /// raw `i32`s off the wire) or from a platform's own output geometry, so
    /// the far edges are computed in `i64` instead of being trusted to fit. A
    /// negative-sized input yields an empty result rather than a rectangle
    /// inside out. The upper half of the size clamp is unreachable -- the
    /// result is contained in both inputs, so it can be no wider than the
    /// narrower of them -- and is there so the `as i32` casts cannot truncate
    /// whatever a future caller passes in.
    pub fn intersection(&self, other: Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right =
            (i64::from(self.x) + i64::from(self.w)).min(i64::from(other.x) + i64::from(other.w));
        let bottom =
            (i64::from(self.y) + i64::from(self.h)).min(i64::from(other.y) + i64::from(other.h));
        let w = (right - i64::from(x)).clamp(0, i64::from(i32::MAX)) as i32;
        let h = (bottom - i64::from(y)).clamp(0, i64::from(i32::MAX)) as i32;
        Rect::new(x, y, w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_rects_intersect_where_they_overlap() {
        let a = Rect::new(0, 0, 100, 100);
        let b = Rect::new(50, 25, 100, 100);
        assert_eq!(a.intersection(b), Rect::new(50, 25, 50, 75));
        assert_eq!(b.intersection(a), Rect::new(50, 25, 50, 75));
    }

    #[test]
    fn a_contained_rect_is_its_own_intersection() {
        let outer = Rect::new(0, 0, 1600, 1000);
        let inner = Rect::new(0, 30, 1600, 970);
        assert_eq!(inner.intersection(outer), inner);
    }

    #[test]
    fn disjoint_rects_intersect_to_nothing() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(100, 100, 10, 10);
        let overlap = a.intersection(b);
        assert_eq!(overlap.size(), Size::new(0, 0));
    }

    #[test]
    fn an_empty_or_inside_out_rect_intersects_to_nothing() {
        let screen = Rect::new(0, 0, 800, 600);
        assert_eq!(
            Rect::new(0, 0, 0, 0).intersection(screen).size(),
            Size::new(0, 0)
        );
        assert_eq!(
            Rect::new(10, 10, -100, -100).intersection(screen).size(),
            Size::new(0, 0)
        );
    }

    /// The far edges are `i64` sums, so the widest rectangles that exist
    /// intersect without overflowing -- `x + w` would wrap in `i32` for every
    /// case here.
    #[test]
    fn extreme_coordinates_do_not_overflow() {
        let screen = Rect::new(0, 0, 1600, 1000);
        // Starts past the screen and runs off the end of the coordinate space
        // (`x + w` wraps to a negative number in `i32`).
        let far = Rect::new(i32::MAX - 1, i32::MAX - 1, i32::MAX, i32::MAX);
        assert_eq!(far.intersection(screen).size(), Size::new(0, 0));
        // Two rectangles that genuinely overlap, both of whose far edges are
        // past `i32::MAX`: the overlap is real and must be found, which a
        // wrapped `x + w` would report as empty.
        let a = Rect::new(1_500_000_000, 0, 1_000_000_000, 10);
        let b = Rect::new(2_000_000_000, 0, 100, 10);
        assert_eq!(a.intersection(b), Rect::new(2_000_000_000, 0, 100, 10));
        // Idempotent even at the edges of the coordinate space.
        let everywhere = Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
        assert_eq!(everywhere.intersection(everywhere), everywhere);
    }

    #[test]
    fn far_edges_saturate_instead_of_overflowing() {
        // `(i32::MAX - 100) + 200` panics in debug and wraps negative in
        // release today.
        let r = Rect::new(i32::MAX - 100, i32::MIN + 100, 200, 200);
        assert_eq!(r.right(), i32::MAX);
        assert_eq!(r.bottom(), i32::MIN + 300);
        // A saturated edge still contains what it should: the last real
        // column is inside, and nothing past the saturation point is.
        assert!(r.contains(Point::new(i32::MAX - 1, i32::MIN + 299)));
        assert!(!r.contains(Point::new(i32::MAX, i32::MIN + 299)));
    }

    #[test]
    fn inset_saturates_the_origin_instead_of_overflowing() {
        // `x + by` past `i32::MAX` panics in debug and wraps in release
        // today, while the `w`/`h` arms already floor at 0.
        let r = Rect::new(i32::MAX - 1, i32::MAX - 1, 10, 10).inset(10);
        assert_eq!((r.x, r.y), (i32::MAX, i32::MAX));
        assert_eq!(r.size(), Size::new(0, 0));
    }

    #[test]
    fn inset_with_an_enormous_margin_leaves_an_empty_rect() {
        // `2 * by` overflows on its own for this input today, before the
        // `.max(0)` floor ever runs.
        let r = Rect::new(0, 0, 1600, 1000).inset(i32::MAX);
        assert_eq!(r.size(), Size::new(0, 0));
        assert_eq!((r.x, r.y), (i32::MAX, i32::MAX));
    }

    #[test]
    fn inset_with_a_negative_margin_still_grows() {
        // Pins the existing grow direction so the saturation above cannot
        // silently turn shrinking inside out.
        assert_eq!(
            Rect::new(10, 10, 100, 100).inset(-5),
            Rect::new(5, 5, 110, 110)
        );
    }
}
