//! Compositor-side rounded window corners (`[appearance] corner_radius`).
//!
//! Windows are client surfaces; the compositor cannot ask the client to draw
//! itself round. What it can do is draw less of the window: each window
//! element is wrapped in [`Rounded`], which cuts the four corner staircases
//! out of every draw and shrinks the element's opaque region to match, so
//! whatever is below shows through the corners. Popups are gathered
//! separately and never wrapped -- menus stay square by decision, and a popup
//! usually extends past its parent's rect, so the parent's clip must not
//! touch it.
//!
//! # Why the opaque region and the draw clip are one atomic change
//!
//! Neither half is correct alone:
//!
//! - Opaque-only (no draw clip) changes no pixel -- the window still paints
//!   its full rect -- while pointlessly compositing what is below. Pure
//!   slowdown, zero visual change.
//! - Draw-clip-only (no opaque change) is a persistent-artifact bug: the
//!   damage tracker trusts the full-rect opaque claim and never redraws what
//!   is below the cut corners, so the corners show stale pixels until some
//!   unrelated damage repaints them.
//!
//! [`Rounded`] therefore always does both, computed from the same radius, so
//! the two cannot drift apart.
//!
//! # Staircase, not a mask
//!
//! The cut is a per-row staircase (`cut_width`), not an anti-aliased mask: a
//! mask needs per-pixel blending the pixman path cannot express through
//! Smithay's renderer-agnostic `draw` (damage rects are the only clip), while
//! the staircase is the same rect list on pixman and GLES -- one
//! implementation, byte-identical pixels on both renderers. The price is
//! 1px-stepped corner edges instead of smooth ones; at the radii people
//! actually configure (8-16px) that reads as rounded at a glance.
//!
//! `radius = 1` cuts nothing: the corner pixel's center is still inside the
//! circle (see `cut_width`), so a radius of 1 renders square. That is the
//! pixel-center rule applied uniformly, not a special case, and it is pinned
//! by test.
//!
//! # One truth, two consumers
//!
//! The window clip ([`Rounded`]) and the focus-ring paint ([`paint_ring`])
//! share [`cut_width`] and [`clip_rect`]: the ring's inner edge is the window's
//! outer edge computed by the same function, so the two coincide exactly and
//! no hairline can open between them. The ring paint consumes rows rather
//! than pixels for the same reason -- `row_cut` is [`cut_width`] with a
//! vertical mirror, not a second circle implementation.
//!
//! # Cost model
//!
//! Per window element per frame, when the radius is non-zero: one damage
//! filter over a handful of rects (amortized allocation-free through a reused
//! scratch buffer) plus ~4 extra pixman composite ops per radius pixel. The
//! opacity loss the ticket prices is separate and real: a window fully behind
//! a rounded one can no longer be skipped, because its corner pixels are now
//! visible. Both are measured, not reasoned about -- see the benchmark.
//!
//! `corner_radius = 0` wraps nothing and paints nothing new: the gather path
//! (`render/elements.rs`'s `window_elements`) pushes each window's own
//! `AsRenderElements` output unwrapped, and the ring keeps its four solid
//! rects, so the default session is byte-identical to before.

use std::cell::RefCell;

use scoot_core::Rect;
use smithay::backend::renderer::Renderer;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{
    Buffer as BufferSpace, Logical, Physical, Point, Rectangle, Scale, Size, Transform,
};

/// The configured radius made effective for one window: clamped into
/// `0..=min(w, h) / 2`, so a radius larger than the window rounds it into a
/// stadium/circle rather than cutting into negative geometry.
///
/// Non-positive window dimensions (a zero-size window) yield 0 -- nothing to
/// round -- instead of a clamp panic. Pure and unit-tested.
pub fn effective_radius(configured: i32, w: i32, h: i32) -> i32 {
    if w <= 0 || h <= 0 {
        return 0;
    }
    configured.max(0).min(w.min(h) / 2)
}

/// How many pixels of one corner row lie outside the quarter circle: `row`
/// counts from the outer edge (`0` is the outermost row), `radius` is in the
/// same physical pixels.
///
/// Pixel-center rule: a pixel draws iff its center is inside the circle, so
/// `cut_width` is the count of pixels in `0..radius` whose center is outside.
/// Closed form of that predicate -- `covers` in the tests below is the
/// independent per-pixel oracle, and a differential test pins the two equal
/// on every radius `0..=64` plus spot larges, so a formula slip fails loudly
/// rather than drawing a wrong staircase.
///
/// The `ceil` cannot produce a tie either way: pixel centers sit on
/// half-integers while the circle equation in those coordinates is `2 (mod
/// 4) == 0 (mod 4)` -- unsatisfiable -- so no pixel center is ever exactly
/// *on* the circle.
pub fn cut_width(radius: i32, row: i32) -> i32 {
    debug_assert!(radius >= 0 && row >= 0 && row < radius);
    let r = f64::from(radius);
    // Pixel-center distance from the outer edge; the circle's center is one
    // radius in from each edge.
    let dy = r - (f64::from(row) + 0.5);
    let dx = (r * r - dy * dy).sqrt();
    // Pixels with index `i` draw where `i + 0.5 >= r - dx`: everything below
    // that bound is cut.
    (r - dx - 0.5).ceil() as i32
}

/// [`cut_width`] for a full-height rect: `y` in `0..h`, mirroring the same
/// staircase at the bottom edge. Rows outside both corner bands cut nothing.
pub fn row_cut(radius: i32, y: i32, h: i32) -> i32 {
    if radius <= 0 {
        return 0;
    }
    if y < radius {
        cut_width(radius, y)
    } else if y >= h - radius {
        cut_width(radius, h - 1 - y)
    } else {
        0
    }
}

/// One window's outer rect in physical output pixels: the placement the
/// layout produced, converted the way surface-element locations are
/// (`to_physical_precise_round`, so the clip and the drawn content agree on
/// where the window starts) with the size scaled and rounded (so they agree
/// on where it ends).
///
/// Shared by the window clip and the ring paint -- see the module doc -- so
/// the two edges coincide by construction rather than by matching
/// conversions written twice.
pub fn clip_rect(placement: Rect, scale: f64) -> Rectangle<i32, Physical> {
    let loc = Point::<i32, _>::from((placement.x, placement.y)).to_physical_precise_round(scale);
    let size = (
        (f64::from(placement.w) * scale).round() as i32,
        (f64::from(placement.h) * scale).round() as i32,
    );
    Rectangle::new(loc, size.into())
}

/// The configured radius in physical pixels, clamped to the clip: what
/// [`Rounded`] and the ring paint both consume. Saturates rather than
/// overflowing on absurd configs (`i32::MAX` at scale 4.0 is past `i32` as a
/// float, and `as` saturates) because the clamp below bounds it to half the
/// window either way.
pub fn physical_radius(configured: i32, clip: Rectangle<i32, Physical>, scale: f64) -> i32 {
    let radius = (f64::from(configured) * scale).round() as i32;
    effective_radius(radius, clip.size.w, clip.size.h)
}

/// One window's corner squares in output coordinates: the conservative
/// opaque-region cut [`Rounded::opaque_regions`] subtracts. Whole squares
/// rather than the staircase -- a superset of the pixels [`cut_width`]
/// removes from the draw, which is the direction that keeps the opaque claim
/// a subset of what the element really paints (see the module doc's atomicity
/// section). Cheaper than per-row rects (four rects, not `4 * radius`) for a
/// precision loss of `(1 - pi/4) * r^2` pixels per corner -- negligible
/// against the window area, and irrelevant to the full-occlusion skip either
/// way (any cut pixel visible defeats the skip under both shapes).
fn corner_squares(clip: Rectangle<i32, Physical>, radius: i32) -> [Rectangle<i32, Physical>; 4] {
    let (x, y, w, h) = (clip.loc.x, clip.loc.y, clip.size.w, clip.size.h);
    let size = (radius, radius).into();
    [
        Rectangle::new((x, y).into(), size),
        Rectangle::new((x + w - radius, y).into(), size),
        Rectangle::new((x, y + h - radius).into(), size),
        Rectangle::new((x + w - radius, y + h - radius).into(), size),
    ]
}

/// A painted focus ring's geometry: where its image sits and how big it is.
///
/// `loc` is the ring image's origin in exact physical pixels (the window
/// origin minus the ring thickness, unrounded -- the element rounds it the
/// same way for every scale), `logical` the image's size in logical pixels
/// (what the element is built at), and `canvas` the image's size in physical
/// pixels (what [`paint_ring`] fills). The canvas is computed with the same
/// rounding Smithay's memory-render element uses internally
/// (`physical_size`: offset-then-round minus origin-round), so the painted
/// pixels and the drawn element agree on every scale -- at scale 1.0 both
/// are exact integers.
pub fn ring_layout(
    placement: Rect,
    thickness: i32,
    scale: f64,
) -> (
    Point<f64, Physical>,
    Size<i32, Logical>,
    Size<i32, Physical>,
) {
    let loc = Point::<f64, Logical>::from((
        f64::from(placement.x - thickness),
        f64::from(placement.y - thickness),
    ))
    .to_physical(scale);
    let logical = Size::<i32, Logical>::from((
        placement.w.saturating_add(thickness.saturating_mul(2)),
        placement.h.saturating_add(thickness.saturating_mul(2)),
    ));
    (loc, logical, element_canvas(loc, logical, scale))
}

/// The physical pixels an element with logical `size` at `loc` actually
/// draws at `scale`: the same offset-then-round minus origin-round Smithay's
/// memory-render element uses internally (`physical_size`), so painted pixels
/// and the drawn element agree on every scale -- at scale 1.0 both are exact
/// integers.
pub fn element_canvas(
    loc: Point<f64, Physical>,
    logical: Size<i32, Logical>,
    scale: f64,
) -> Size<i32, Physical> {
    let end = (logical.to_f64().to_physical(scale).to_point() + loc).to_i32_round();
    (end - loc.to_i32_round()).to_size()
}

/// Paints one window's rounded focus ring into `pixels`: the band between the
/// outer rounded rect (`outer`, radius `radius_outer`) and the window's own
/// rounded rect (`inner`, radius `radius_inner`), in `color`, transparent
/// everywhere else. Both rects are in canvas coordinates (canvas origin is
/// `(0, 0)`); rows neither rect reaches paint nothing.
///
/// Row spans, not per-pixel tests: each row fills at most two runs computed
/// from [`row_cut`], so this is the same staircase the window clip draws --
/// the ring's inner edge and the window's outer edge are the same rects, and
/// no hairline can open between them. Runs only on repaints (resize, radius,
/// width or color change), never per frame.
///
/// The ring is painted as two strips (top and bottom -- see
/// `decorations.rs`), not one full-window image: a full-canvas buffer is
/// mostly transparent middle, and compositing it costs a full-window blend
/// every frame for pixels that change nothing. The strips cover only rows
/// that actually hold ring pixels; the straight side runs stay solid rects.
///
/// `color` is one premultiplied `Argb8888` pixel in little-endian order (see
/// `Color::to_argb8888`); the buffer is `canvas_w * canvas_h * 4` bytes,
/// row-major. The caller sizes both from [`ring_layout`], and the debug
/// assert pins that contract wherever tests repaint.
/// What [`paint_ring`] fills: the canvas size plus the two rounded rects
/// (in canvas coordinates) whose band is the ring.
pub struct RingPaint {
    /// Canvas size in physical pixels (what the pixel buffer holds).
    pub canvas: Size<i32, Physical>,
    /// The ring's outer edge and its radius.
    pub outer: Rectangle<i32, Physical>,
    pub radius_outer: i32,
    /// The window's edge and its radius.
    pub inner: Rectangle<i32, Physical>,
    pub radius_inner: i32,
}

pub fn paint_ring(pixels: &mut [u8], paint: &RingPaint, color: [u8; 4]) {
    let (canvas_w, canvas_h) = (paint.canvas.w, paint.canvas.h);
    debug_assert_eq!(
        pixels.len(),
        canvas_w as usize * canvas_h as usize * 4,
        "paint_ring's buffer must be exactly the canvas"
    );
    let row_bytes = canvas_w as usize * 4;
    for y in 0..canvas_h {
        let Some((outer_left, outer_right)) = row_span(paint.radius_outer, y, paint.outer) else {
            continue;
        };
        let spans: [(i32, i32); 2] = match row_span(paint.radius_inner, y, paint.inner) {
            Some((inner_left, inner_right)) => {
                [(outer_left, inner_left), (inner_right, outer_right)]
            }
            None => [(outer_left, outer_right), (0, 0)],
        };
        let base = y as usize * row_bytes;
        for (start, end) in spans {
            // A miscomputed span must clip to the canvas, never index out of
            // it. Spans are inside by construction (see `row_span`), so in
            // practice these clamps are no-ops that read as the contract.
            let (start, end) = (start.max(0), end.min(canvas_w));
            if start >= end {
                continue;
            }
            for pixel in
                pixels[base + start as usize * 4..base + end as usize * 4].chunks_exact_mut(4)
            {
                pixel.copy_from_slice(&color);
            }
        }
    }
}

/// One row's span inside a rounded rect, or `None` when the row misses the
/// rect entirely (above/below it, or fully cut away at a corner).
fn row_span(radius: i32, y: i32, rect: Rectangle<i32, Physical>) -> Option<(i32, i32)> {
    let yi = y - rect.loc.y;
    if yi < 0 || yi >= rect.size.h {
        return None;
    }
    let cut = row_cut(radius, yi, rect.size.h);
    let (left, right) = (rect.loc.x + cut, rect.loc.x + rect.size.w - cut);
    if left >= right {
        None
    } else {
        Some((left, right))
    }
}

/// The staircase cut as damage rects in output coordinates: one 1px-tall rect
/// per corner row with a non-zero cut. Lazy -- no allocation for the row
/// list -- since this runs per element per frame.
fn corner_rows(
    clip: Rectangle<i32, Physical>,
    radius: i32,
) -> impl Iterator<Item = Rectangle<i32, Physical>> {
    let (x, y, w, h) = (clip.loc.x, clip.loc.y, clip.size.w, clip.size.h);
    (0..radius)
        .flat_map(move |row| {
            let cut = cut_width(radius, row);
            if cut <= 0 {
                return None;
            }
            let size = (cut, 1).into();
            Some(
                [
                    Rectangle::new((x, y + row).into(), size),
                    Rectangle::new((x + w - cut, y + row).into(), size),
                    Rectangle::new((x, y + h - 1 - row).into(), size),
                    Rectangle::new((x + w - cut, y + h - 1 - row).into(), size),
                ]
                .into_iter(),
            )
        })
        .flatten()
}

/// A window surface element with its corners cut out. Wraps one
/// `WaylandSurfaceRenderElement` from the window's *toplevel* tree; popup
/// trees are gathered separately and never wrapped.
///
/// `clip` is the window's outer rect in output-physical pixels and `radius`
/// the configured radius in the same units; the constructor clamps the
/// radius to the clip, so a stored `Rounded` always holds a valid pair.
/// Subsurface elements smaller than, or offset inside, the clip work
/// unchanged: the corner rects simply miss them (or intersect partially),
/// because every cut is computed in output coordinates and converted to the
/// element's own frame only for the opaque subtraction.
pub struct Rounded<E> {
    inner: E,
    clip: Rectangle<i32, Physical>,
    radius: i32,
    /// The corner-filtered damage, reused across frames: `draw` takes `&self`
    /// but must hand the inner draw a filtered `&[Rectangle]`, so the storage
    /// has to live here. `take`n and restored per draw -- capacity persists,
    /// so the steady state allocates nothing. Never borrowed reentrantly:
    /// elements draw sequentially, and the inner draw never calls back in.
    scratch: RefCell<Vec<Rectangle<i32, Physical>>>,
}

impl<E> Rounded<E> {
    /// Wraps `inner`, clamping `radius` to the clip (see [`effective_radius`]).
    /// A clamped-to-zero radius still wraps -- the caller decides whether to
    /// wrap at all (it shouldn't: an unwrapped element is cheaper and keeps
    /// the damage tracker's element identity untouched).
    pub fn new(inner: E, clip: Rectangle<i32, Physical>, radius: i32) -> Self {
        let radius = effective_radius(radius, clip.size.w, clip.size.h);
        Self {
            inner,
            clip,
            radius,
            scratch: RefCell::new(Vec::new()),
        }
    }

    /// The clamped radius this wrapper actually cuts. Test-only: production
    /// code picks wrapped-vs-plain before constructing.
    #[cfg(test)]
    fn radius(&self) -> i32 {
        self.radius
    }
}

impl<E: Element> Element for Rounded<E> {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn src(&self) -> Rectangle<f64, BufferSpace> {
        self.inner.src()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        // The bounding box, unchanged: damage stays the bounding box (correct
        // by construction -- the corners repaint through to whatever is
        // below, since the draw clip below applies on every draw no matter
        // whose damage it is).
        self.inner.geometry(scale)
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        if self.radius <= 0 {
            return self.inner.opaque_regions(scale);
        }
        let opaque = self.inner.opaque_regions(scale);
        if opaque.is_empty() {
            return opaque;
        }
        // Element-relative corner squares: the opaque regions are relative
        // to the element, the clip is in output coordinates.
        let offset = self.inner.geometry(scale).loc;
        let squares = corner_squares(self.clip, self.radius)
            .into_iter()
            .map(|mut rect| {
                rect.loc -= offset;
                rect
            });
        Rectangle::subtract_rects_many(opaque.iter().copied(), squares)
            .into_iter()
            .collect()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }

    fn is_framebuffer_effect(&self) -> bool {
        self.inner.is_framebuffer_effect()
    }
}

impl<R: Renderer, E: RenderElement<R>> RenderElement<R> for Rounded<E> {
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, BufferSpace>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        if self.radius <= 0 {
            return self
                .inner
                .draw(frame, src, dst, damage, opaque_regions, cache);
        }
        // `damage` arrives relative to the element (the damage tracker
        // subtracts the element geometry before calling draw), while the clip
        // is in output coordinates: convert the corner rows into the
        // element's frame. `dst` is that geometry -- the tracker passes the
        // same rect it positioned the element at -- so no second
        // `geometry()` call is needed.
        let offset = dst.loc;
        let rows = corner_rows(self.clip, self.radius).map(|mut rect| {
            rect.loc -= offset;
            rect
        });
        let mut filtered = std::mem::take(&mut *self.scratch.borrow_mut());
        filtered.extend(damage.iter().copied());
        filtered = Rectangle::subtract_rects_many_in_place(filtered, rows);
        let result = self
            .inner
            .draw(frame, src, dst, &filtered, opaque_regions, cache);
        *self.scratch.borrow_mut() = filtered;
        result
    }

    #[inline]
    fn underlying_storage(&self, renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        self.inner.underlying_storage(renderer)
    }

    fn capture_framebuffer(
        &self,
        frame: &mut <R>::Frame<'_, '_>,
        src: Rectangle<f64, BufferSpace>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), <R>::Error> {
        self.inner.capture_framebuffer(frame, src, dst, cache)
    }
}

#[cfg(test)]
mod tests;
