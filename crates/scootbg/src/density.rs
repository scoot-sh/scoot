//! How many device pixels a wallpaper buffer gets: the arithmetic behind
//! drawing at an output's real pixels, fractional scales included. Pure,
//! with no Wayland objects, so every rounding case is a unit test.
//!
//! The compositor says what scale a surface should be drawn at in up to
//! three ways; the best one known wins ([`Preferred::scale`]), unless it is
//! stale (below):
//!
//! 1. **`wp_fractional_scale_v1.preferred_scale`**, in 120ths (180 is
//!    1.5). Used only where scootbg can act on it, which takes a
//!    `wp_viewporter` too: the buffer is the surface's logical size times
//!    the scale, each side rounded halfway away from zero (the protocol's
//!    rule, [`scaled_length`]), attached at buffer scale 1, and the
//!    viewport's destination is the logical size.
//! 2. **`wl_surface.preferred_buffer_scale`** (`wl_surface` v6), an
//!    integer: the buffer is the logical size times it, attached with
//!    `set_buffer_scale`.
//! 3. **`wl_output.scale`**, the same way. At a fractional scale it is the
//!    scale rounded up, so the buffer is larger than the output and the
//!    compositor scales it down: sharp, but not device-exact, and more
//!    pixels than the output has.
//!
//! Without a usable fraction, the integer scale is the larger of 2 and 3
//! ([`Preferred::integer`]).
//!
//! ## A stale scale
//!
//! The surface's scales and `wl_output.scale` arrive separately, and can
//! disagree for a while: sway sends a changed output's `wl_output.scale`
//! at once, but wlroots re-sends a surface's `preferred_scale` and
//! `preferred_buffer_scale` only while the surface is on screen, so one
//! showing nothing (cleared, or never set), or fully covered, keeps the
//! old values until it is shown. A buffer drawn for more pixels than the
//! output has is scaled down (sharp, only larger), where one drawn for
//! fewer would be stretched (a blur), so a scale is distrusted only when it
//! would make the buffer too small, in two ways:
//!
//! - **Against `wl_output.scale`.** Every compositor checked sends it as
//!   the fraction rounded up, so a surface scale below it is stale, or it
//!   is; which cannot be known, and the larger wins ([`Preferred::scale`]).
//! - **Against the output's mode** ([`Scale::falls_short`]). 1.25 and 1.5
//!   both round up to 2, so the check above cannot tell them apart; but the
//!   surface covers the whole output, so its logical size times the right
//!   fraction is the mode's size to within the rounding (a pixel or two,
//!   see the tests), where a stale 1.25 on sway's 1066-wide surface at 1.5
//!   gives 1333 for a 1600-pixel mode. A fraction that falls short like
//!   that gives way to the integer scale.
//!
//! Either way the surface is drawn larger than needed, not stretched, until
//! the compositor's next event settles it and the surface is redrawn: the
//! wallpaper is on screen when `set` answers, but not device-exact until
//! then. A compositor whose fraction always overshoots the mode (scoot at
//! 1.33, `docs/backlog/core/fractional-scale-in-120ths.md`) keeps it: the
//! buffer is scaled down a little either way, and the fraction's is the
//! smaller one.
//!
//! ## Why the buffer is not snapped to the output's mode
//!
//! A surface covering a 2560-pixel-wide output at 1.5 is 1707 logical
//! pixels wide on a compositor that rounds the logical size up, and 1707 ×
//! 1.5 = 2560.5 rounds to 2561, a pixel wider than the mode. It is
//! tempting to draw 2560 instead. But what makes a buffer exact is that it
//! matches the rectangle the compositor draws it into, and both compositors
//! checked size that rectangle as `round(logical × scale)` too:
//!
//! - Smithay (scoot): `WaylandSurfaceRenderElement::size` rounds the
//!   destination's physical size, so 1067 logical at 1.5 is drawn 1601
//!   pixels wide on a 1600-pixel output, and the last column is clipped;
//! - wlroots (sway): the logical size is the mode divided by the scale and
//!   *truncated* (`wlr_output_effective_resolution`), so the same output is
//!   1066 wide, and `scale_length` draws it 1599 pixels wide.
//!
//! A 1601- or 1599-pixel buffer lands one to one on those; a 1600-pixel one
//! would be stretched or squeezed by a pixel across the whole width, which
//! is exactly the resampling blur this is here to avoid. So the protocol's
//! rounding stands, and the screenshots in `tests/scale.rs` hold it to
//! that on both compositors.

#[cfg(test)]
mod tests;

use std::fmt;

use crate::outputs::Size;

/// `wp_fractional_scale_v1` scales are in 120ths.
pub const DENOMINATOR: u32 = 120;

/// The scale a surface is drawn at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    /// `wl_surface.set_buffer_scale` with this factor, never 0.
    Integer(u32),
    /// `wp_fractional_scale_v1`, in 120ths, never 0: a buffer at buffer
    /// scale 1 under a viewport.
    Fractional(u32),
}

impl Scale {
    /// The buffer to draw a surface of `size` logical pixels at this scale.
    /// `None` if a side overflows `u32`; the buffer type then refuses
    /// anything past `i32::MAX` bytes anyway.
    pub fn buffer(self, size: Size) -> Option<Buffer> {
        match self {
            Self::Integer(factor) => {
                let factor = factor.max(1);
                Some(Buffer {
                    dims: (
                        size.width.checked_mul(factor)?,
                        size.height.checked_mul(factor)?,
                    ),
                    scale: factor,
                })
            }
            Self::Fractional(v120) => Some(Buffer {
                dims: (
                    scaled_length(size.width, v120)?,
                    scaled_length(size.height, v120)?,
                ),
                scale: 1,
            }),
        }
    }

    /// The scale as a number.
    pub fn value(self) -> f64 {
        match self {
            Self::Integer(factor) => f64::from(factor),
            Self::Fractional(v120) => f64::from(v120) / f64::from(DENOMINATOR),
        }
    }

    /// Whether a buffer at this scale for a surface of `logical` pixels
    /// (an axis of 0: not known, not checked) falls short of `device`, the
    /// output's mode rotated as the output is, by more than the rounding
    /// can explain: by at least the scale plus half a pixel on an axis.
    /// A surface covering the whole output never does when the compositor
    /// renders at exactly `v120`/120 (a compositor that rounds the logical
    /// size down, as wlroots does, falls short by less than that); a stale
    /// fraction does. So can a compositor rendering at a scale *below* the
    /// 120th it sends (scoot at 1.254 sends 150): that falls back to the
    /// integer scale for as long as the scale is set, larger and sharp but
    /// about 2.5× the pixels. One *above* it (1.33, sent as 160) keeps the
    /// fraction. Never true for an integer scale: that is the
    /// fallback, and is checked against `wl_output.scale` instead.
    pub fn falls_short(self, logical: Size, device: Size) -> bool {
        let Self::Fractional(v120) = self else {
            return false;
        };
        let short = |length: u32, device: u32| {
            if length == 0 {
                return false;
            }
            let Some(drawn) = scaled_length(length, v120) else {
                return false;
            };
            // `device − drawn ≥ v120/120 + 1/2`, in 120ths.
            let deficit = u64::from(device.saturating_sub(drawn)) * u64::from(DENOMINATOR);
            deficit >= u64::from(v120) + u64::from(DENOMINATOR / 2)
        };
        short(logical.width, device.width) || short(logical.height, device.height)
    }

    /// The logical size of an output whose mode, rotated as the output is,
    /// is `device` pixels, as a compositor would work it out. Only for a
    /// surface the compositor left the size of to scootbg (a `configure`
    /// of 0), and for `query` before any `configure`: compositors round
    /// differently (scoot up, wlroots down), so this can be a pixel off
    /// either way. An integer scale divides and rounds down, as wlroots
    /// does, exact at any integer scale; a fractional one rounds up, so a
    /// surface sized by it covers the output rather than leaving a gap.
    /// Never 0 on a side that is not 0.
    pub fn logical(self, device: Size) -> Size {
        let side = |length: u32| -> u32 {
            match self {
                Self::Integer(factor) => (length / factor.max(1)).max(length.min(1)),
                Self::Fractional(v120) => {
                    let v120 = u64::from(v120.max(1));
                    let up = (u64::from(length) * u64::from(DENOMINATOR)).div_ceil(v120);
                    // At most `length × 120` over at least 1: can exceed
                    // `u32` only below scale 1/120, which is not a scale.
                    u32::try_from(up).unwrap_or(u32::MAX)
                }
            }
        };
        Size {
            width: side(device.width),
            height: side(device.height),
        }
    }
}

/// For `query`: a whole number as an integer (`2`, and `2` for 240/120),
/// anything else as a decimal (`1.5`).
impl serde::Serialize for Scale {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match *self {
            Self::Integer(factor) => serializer.serialize_u32(factor),
            Self::Fractional(v120) if v120 % DENOMINATOR == 0 => {
                serializer.serialize_u32(v120 / DENOMINATOR)
            }
            Self::Fractional(_) => serializer.serialize_f64(self.value()),
        }
    }
}

impl fmt::Display for Scale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value())
    }
}

/// A buffer to draw: its size in pixels, and the `wl_surface` buffer scale
/// it is attached at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Buffer {
    pub dims: (u32, u32),
    pub scale: u32,
}

impl Buffer {
    /// Whether, attached at its buffer scale, it is `size` logical pixels
    /// with no viewport: anything else needs a viewport's destination to
    /// size it (a 1×1 color, or a fractional-scale buffer).
    pub fn fits(&self, size: Size) -> bool {
        let scale = self.scale.max(1);
        size.width.checked_mul(scale) == Some(self.dims.0)
            && size.height.checked_mul(scale) == Some(self.dims.1)
    }
}

/// `length` logical pixels at `v120`/120, in buffer pixels, rounded halfway
/// away from zero as `wp_fractional_scale_v1` specifies (the lengths are
/// never negative, so that is halves up), and at least 1: a buffer side of
/// 0 is no buffer. `None` past `u32`.
pub fn scaled_length(length: u32, v120: u32) -> Option<u32> {
    let product = u64::from(length) * u64::from(v120.max(1));
    let rounded = (product + u64::from(DENOMINATOR / 2)) / u64::from(DENOMINATOR);
    u32::try_from(rounded.max(1)).ok()
}

/// What the compositor said about how to draw one surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Preferred {
    /// `wp_fractional_scale_v1.preferred_scale`, in 120ths, never 0. Only
    /// ever set where the surface has a fractional-scale object, which
    /// scootbg makes only with a viewporter to use it with.
    pub fractional: Option<u32>,
    /// `wl_surface.preferred_buffer_scale` (v6), never 0.
    pub buffer_scale: Option<u32>,
}

impl Preferred {
    /// The best scale known: the fractional one, else the larger of the
    /// surface's integer one and the output's (`wl_output.scale`). A
    /// fraction whose rounding up is below `wl_output.scale` is taken as
    /// stale and the integer used (see the module docs). The check against
    /// the output's mode is [`Scale::falls_short`], which needs the
    /// configured size (`Output::scale`).
    pub fn scale(&self, output_scale: u32) -> Scale {
        match self.fractional {
            Some(v120) if v120.div_ceil(DENOMINATOR) >= output_scale.max(1) => {
                Scale::Fractional(v120)
            }
            _ => self.integer(output_scale),
        }
    }

    /// The integer scale: the larger of the surface's and the output's.
    /// What a fraction found stale gives way to.
    pub fn integer(&self, output_scale: u32) -> Scale {
        Scale::Integer(self.buffer_scale.unwrap_or(1).max(output_scale.max(1)))
    }

    /// `preferred_scale`: a scale of 0 is a broken compositor, and ignored
    /// rather than let it reach a size. Returns whether it changed.
    pub fn set_fractional(&mut self, v120: u32) -> bool {
        if v120 == 0 {
            return false;
        }
        self.fractional.replace(v120) != Some(v120)
    }

    /// `preferred_buffer_scale`: below 1 is ignored, as for
    /// `wl_output.scale`. Returns whether it changed.
    pub fn set_buffer_scale(&mut self, factor: i32) -> bool {
        match u32::try_from(factor) {
            Ok(factor @ 1..) => self.buffer_scale.replace(factor) != Some(factor),
            _ => false,
        }
    }
}
