//! The 24-unit design grid every cursor shape is authored in.
//!
//! Coordinates in the family modules are in 24ths of the bitmap edge:
//! `u = size / 24` is one grid unit, so a shape drawn at 16px is the same
//! geometry scaled by `16/24`, not a separate drawing. What was previously
//! written as ad-hoc fractions of the bitmap (`6.0 * size / 16`, `size / 8`)
//! is now written as grid units (`9.0 * u`, `3.0 * u`): identical pixels,
//! one authoring space.
//!
//! # Why 24
//!
//! The user asked to "make at 24 and downsize" with the default staying 16
//! logical px. 24 is the smallest grid where every family feature lands on
//! a representable coordinate: the arrow's 45-degree hypotenuse, the I-beam
//! serifs, the paw toe arc and the hand finger pitch are all multiples of
//! half a unit there, while at 16 they straddle pixel cracks. Authoring at
//! 24 and scaling down keeps those relations exact at every target size.
//!
//! # Rendering, not resampling
//!
//! Because the shapes are SDF, "downsize" never means rendering a 24px
//! bitmap and resampling it: the same grid geometry is rasterised at the
//! target pixel size with grid-aware hinting (half-pixel `c`, `span`).
//! A 24-render-then-resample experiment was tried against native 16 renders
//! (see the round-5 report): the resampled bitmaps are softer on every
//! shape -- one extra interpolation of an already anti-aliased edge -- so
//! the shipped path stays native.
//!
//! # Per-size simplification, not silent blur
//!
//! Below 16px some 24-grid detail is sub-pixel and is dropped explicitly
//! instead of being left to dissolve into blends:
//!
//! - strokes never go below 1px (`MIN_STROKE`);
//! - the paw's toes drop to the pad alone once a toe's radius would fall
//!   below [`TOE_DROP_BELOW`] (the 8px pointer badge);
//! - the text serifs shorten below 16px and drop to a stem-only beam only
//!   if a future size needs it (currently they survive at 0.7x);
//! - hand-tuned 16px overrides exist ONLY where a plain scale of the 24
//!   design loses legibility (the F4 groove, the badge tucks, the Help
//!   stem): each is commented at its site and judged in the report.
//!
//! # Concept-sheet proportions (measured, not traced)
//!
//! All values below are ratios read off the concept sheet's 24px cells
//! against this family's geometry at 24px. Nothing is pixel-copied; the
//! adjustments are proportional moves inside our own SDF geometry.
//!
//! | Feature | Concept | Ours (round 5) |
//! |---|---|---|
//! | Arrow height : width | ~1.9 | ~1.9 (unchanged) |
//! | Arrow tail length : height | ~0.42 | ~0.40 (tail hint stripe marks the end) |
//! | Accent tail hint placement | last 8% of tail | last 7% (unchanged) |
//! | Hand cuff height : hand height | ~0.16 | 2/14.5 ~= 0.14 at 16px grid |
//! | Paw pad : toe diameter | ~1.8 | 0.72/0.40 = 1.8 (toes widened to a broader arc) |
//! | I-beam serif : stem length | ~0.30 | ~0.30 at 24px |

/// One grid unit at the target bitmap size: `size / 24`.
pub(crate) fn unit(size: i32) -> f64 {
    f64::from(size) / 24.0
}

/// The design edge all shapes are authored against.
pub(crate) const GRID: f64 = 24.0;

/// No stroke is ever drawn below this width in pixels: sub-pixel line art
/// is dropped or merged, never left to blur.
pub(crate) const MIN_STROKE: f64 = 1.0;

/// A paw toe narrower than this radius in pixels is dropped, leaving the
/// pad alone (the 8px pointer badge).
pub(crate) const TOE_DROP_BELOW: f64 = 0.6;
