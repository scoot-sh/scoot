//! Text: measuring and drawing a line with `ab_glyph`, through a bounded
//! glyph cache. Grayscale coverage, no hinting, no shaping and no kerning
//! (one glyph per character, advanced by its width), as M0 chose for a bar
//! whose first text is a clock (`docs/scootbar/backlog/icons-and-fonts.md`
//! takes up fallback fonts and hinting).
//!
//! ## Sizes
//!
//! A size is the em in device pixels (the logical `--font-size` times the
//! output's scale). `ab_glyph`'s `PxScale` is the ascent-to-descent height,
//! not the em, so it is converted: `em × height ÷ units per em`, as M0's
//! spike does.
//!
//! ## The cache
//!
//! A glyph is rasterized the first time it is drawn at a size, and its
//! coverage kept: one arena of bytes, one sorted list of entries. Both are
//! bounded ([`MAX_GLYPHS`], [`MAX_ARENA`]); when either would be passed the
//! whole cache is dropped and refilled from what is drawn next, so an
//! endless stream of new characters (a window title, later) costs a
//! bounded amount of memory, not a growing one. A glyph too big to cache
//! ([`MAX_GLYPH`], 1 MiB: a glyph at a 1024-pixel em, `--font-size 256` at
//! scale 4, still fits) is rasterized straight onto the canvas each time.
//!
//! ## The rasterizer's own allocation
//!
//! `ab_glyph` rasterizes into a buffer of one `f32` per pixel of the
//! glyph's bounds, allocated for each rasterization. The bounds come from
//! the font file, so a hostile or broken font (coordinates near `i16::MAX`
//! at a small units-per-em) could ask for gigabytes. A glyph whose bounds
//! pass [`MAX_RASTER_SIDE`] on a side or [`MAX_RASTER`] pixels in all is
//! therefore not rasterized: it draws nothing. Real text stays far inside
//! both (a glyph at a 1024-pixel em is about 1,000 × 1,300).
//!
//! Control characters are neither measured nor drawn.

use ab_glyph::{Font, FontArc, GlyphId, OutlinedGlyph, PxScale, ScaleFont, point};

use crate::color::Color;
use crate::paint::{Canvas, Span};

#[cfg(test)]
mod tests;

/// Cached glyphs at most, across every size.
pub const MAX_GLYPHS: usize = 512;
/// Coverage bytes cached at most.
pub const MAX_ARENA: usize = 4 * 1024 * 1024;
/// The largest glyph cached, in coverage bytes (a 1024-pixel square).
pub const MAX_GLYPH: usize = 1024 * 1024;
/// The longest side, in pixels, of a glyph that is rasterized at all.
pub const MAX_RASTER_SIDE: u32 = 4096;
/// The most pixels a glyph that is rasterized at all may cover: the
/// rasterizer's buffer is 4 bytes each, so at most 16 MiB, transiently.
pub const MAX_RASTER: usize = 4 * 1024 * 1024;

/// One cached glyph: its coverage at `arena[offset..offset + w × h]`, and
/// where its top-left pixel sits from the pen on the baseline.
#[derive(Debug, Clone, Copy)]
struct Cached {
    key: u64,
    width: u32,
    height: u32,
    left: i32,
    top: i32,
    offset: usize,
}

/// A glyph ready to draw.
enum Glyph {
    Cached(Cached),
    Uncached(OutlinedGlyph),
}

/// A glyph's bounds as whole pixels, if they are finite and within the
/// rasterizer's bounds ([`MAX_RASTER_SIDE`], [`MAX_RASTER`]).
fn raster_size(width: f32, height: f32) -> Option<(u32, u32)> {
    let side = |length: f32| {
        (length.is_finite() && (0.0..=MAX_RASTER_SIDE as f32).contains(&length))
            .then_some(length as u32)
    };
    let (width, height) = (side(width)?, side(height)?);
    (width as usize * height as usize <= MAX_RASTER).then_some((width, height))
}

/// A font and its glyph cache.
pub struct Text {
    font: FontArc,
    /// Sorted by `key`.
    entries: Vec<Cached>,
    arena: Vec<u8>,
}

/// A size's vertical metrics, in device pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// Above the baseline, positive.
    pub ascent: f32,
    /// Below the baseline, negative.
    pub descent: f32,
}

impl Metrics {
    /// The baseline that centers a line vertically in `height` rows.
    pub fn baseline(self, height: u32) -> i64 {
        let line = self.ascent - self.descent;
        let top = (height as f32 - line) / 2.0;
        (top + self.ascent).round() as i64
    }
}

impl Text {
    pub fn new(font: FontArc) -> Self {
        Self {
            font,
            entries: Vec::new(),
            arena: Vec::new(),
        }
    }

    fn scale(&self, em: f32) -> PxScale {
        let units = self.font.units_per_em().unwrap_or(1000.0).max(1.0);
        PxScale::from(em.max(0.0) * self.font.height_unscaled() / units)
    }

    pub fn metrics(&self, em: f32) -> Metrics {
        let scaled = self.font.as_scaled(self.scale(em));
        Metrics {
            ascent: scaled.ascent(),
            descent: scaled.descent(),
        }
    }

    /// The width, in device pixels rounded up, of `icon` (then a space)
    /// and `text` at `em`.
    pub fn measure(&self, icon: Option<char>, text: &str, em: f32) -> u32 {
        let scaled = self.font.as_scaled(self.scale(em));
        let mut width = 0.0f32;
        for c in chars(icon, text) {
            width += scaled.h_advance(self.font.glyph_id(c));
        }
        // Finite and non-negative for any real font; `as` saturates the
        // rest.
        width.ceil().max(0.0) as u32
    }

    /// One character's advance at `em`, in device pixels, unrounded: what
    /// [`Text::measure`] sums and [`Text::draw`] steps its pen by. A caller
    /// walking the same characters in the same order lands exactly where
    /// the draw lands, which is how the workspaces module's pill rects
    /// match its ink.
    #[allow(dead_code)] // Only the workspaces module walks advances.
    pub fn advance(&self, c: char, em: f32) -> f32 {
        let scaled = self.font.as_scaled(self.scale(em));
        scaled.h_advance(self.font.glyph_id(c))
    }

    /// Draws `icon` and `text` at `em` in `color`, the pen starting at `x`
    /// on the baseline `baseline`, clipped to `clip`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        canvas: &mut Canvas<'_>,
        icon: Option<char>,
        text: &str,
        em: f32,
        x: i64,
        baseline: i64,
        color: Color,
        clip: Span,
    ) {
        let scale = self.scale(em);
        let mut pen = x as f32;
        for c in chars(icon, text) {
            let id = self.font.glyph_id(c);
            let advance = self.font.as_scaled(scale).h_advance(id);
            let origin = pen.round() as i64;
            if origin >= i64::from(clip.end()) {
                break;
            }
            self.draw_glyph(canvas, id, scale, origin, baseline, color, clip);
            pen += advance;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_glyph(
        &mut self,
        canvas: &mut Canvas<'_>,
        id: GlyphId,
        scale: PxScale,
        x: i64,
        baseline: i64,
        color: Color,
        clip: Span,
    ) {
        let key = u64::from(id.0) << 32 | u64::from(scale.y.to_bits());
        let found = match self.entries.binary_search_by_key(&key, |e| e.key) {
            Ok(index) => self.entries.get(index).copied().map(Glyph::Cached),
            Err(index) => self.fill(key, id, scale, index),
        };
        match found {
            Some(Glyph::Cached(glyph)) => {
                let len = glyph.width as usize * glyph.height as usize;
                let Some(coverage) = self.arena.get(glyph.offset..glyph.offset + len) else {
                    return;
                };
                let rows = coverage.chunks_exact(glyph.width.max(1) as usize);
                for (gy, row) in rows.enumerate() {
                    let y = baseline + i64::from(glyph.top) + gy as i64;
                    for (gx, &c) in row.iter().enumerate() {
                        let px = x + i64::from(glyph.left) + gx as i64;
                        canvas.blend(px, y, c, color, clip);
                    }
                }
            }
            // Too big to cache (but not to rasterize): straight to the
            // canvas.
            Some(Glyph::Uncached(outline)) => {
                let bounds = outline.px_bounds();
                let (left, top) = (bounds.min.x as i64, bounds.min.y as i64);
                outline.draw(|gx, gy, c| {
                    canvas.blend(
                        x + left + i64::from(gx),
                        baseline + top + i64::from(gy),
                        coverage(c),
                        color,
                        clip,
                    );
                });
            }
            // Blank (a space), or too big to rasterize at all.
            None => {}
        }
    }

    fn outline(&self, id: GlyphId, scale: PxScale) -> Option<OutlinedGlyph> {
        self.font
            .outline_glyph(id.with_scale_and_position(scale, point(0.0, 0.0)))
    }

    /// Rasterizes glyph `id` at `scale` into the cache at `index` (where a
    /// search for `key` found its place). The outline alone for a glyph
    /// too big to cache; `None` for one with no outline (a space) or too
    /// big to rasterize (see the module docs).
    fn fill(&mut self, key: u64, id: GlyphId, scale: PxScale, index: usize) -> Option<Glyph> {
        let outline = self.outline(id, scale)?;
        let bounds = outline.px_bounds();
        let (width, height) = raster_size(bounds.width(), bounds.height())?;
        let len = width as usize * height as usize;
        if len > MAX_GLYPH {
            return Some(Glyph::Uncached(outline));
        }
        let mut index = index;
        if self.entries.len() >= MAX_GLYPHS || self.arena.len() + len > MAX_ARENA {
            self.entries.clear();
            self.arena.clear();
            index = 0;
        }
        let offset = self.arena.len();
        self.arena.resize(offset + len, 0);
        let target = self.arena.get_mut(offset..offset + len)?;
        outline.draw(|gx, gy, c| {
            if gx < width {
                if let Some(slot) = target.get_mut(gy as usize * width as usize + gx as usize) {
                    *slot = coverage(c);
                }
            }
        });
        let glyph = Cached {
            key,
            width,
            height,
            left: bounds.min.x as i32,
            top: bounds.min.y as i32,
            offset,
        };
        self.entries.insert(index.min(self.entries.len()), glyph);
        Some(Glyph::Cached(glyph))
    }

    /// Glyphs cached now, for tests and the bench.
    #[cfg(test)]
    pub fn cached(&self) -> (usize, usize) {
        (self.entries.len(), self.arena.len())
    }
}

/// `icon`, a space after it if there is text, then `text`, without control
/// characters.
fn chars(icon: Option<char>, text: &str) -> impl Iterator<Item = char> + '_ {
    let gap = icon.filter(|_| !text.is_empty()).map(|_| ' ');
    icon.into_iter()
        .chain(gap)
        .chain(text.chars())
        .filter(|c| !c.is_control())
}

/// `ab_glyph`'s coverage, 0 to 1, as a byte.
fn coverage(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}
