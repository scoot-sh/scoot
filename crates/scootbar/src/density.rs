//! How many device pixels a bar buffer gets: drawing at an output's real
//! pixels, fractional scales included. Pure, so every rounding case is a
//! unit test.
//!
//! Ported from scootbg's `density.rs`, which has the full argument and the
//! screenshots behind it; what is left out and why is said below. The
//! compositor says what scale to draw at in up to three ways, and the best
//! one known wins ([`Preferred::scale`]):
//!
//! 1. **`wp_fractional_scale_v1.preferred_scale`**, in 120ths (180 is
//!    1.5), used only with a `wp_viewporter` to act on it: the buffer is
//!    the surface's logical size times the scale, each side rounded
//!    halfway away from zero (the protocol's rule, [`scaled_length`]), and
//!    the viewport's destination is the logical size.
//! 2. **`wl_surface.preferred_buffer_scale`** (`wl_surface` v6), an
//!    integer.
//! 3. **`wl_output.scale`**, the fractional scale rounded up.
//!
//! A fraction whose rounding up is below `wl_output.scale` is taken as
//! stale (wlroots re-sends a surface's scale only while it is on screen)
//! and the integer scale is used instead: drawn larger and scaled down,
//! sharp, rather than stretched.
//!
//! Left out from scootbg: its second staleness check against the output's
//! mode (`falls_short`), which rests on the surface covering the whole
//! output. A bar covers one strip of it, so the check has nothing to
//! compare its height with; and a bar, unlike a wallpaper, is on screen
//! whenever it is drawn, which is what makes wlroots re-send the scale.

#[cfg(test)]
mod tests;

use std::fmt;

use crate::outputs::Size;

/// `wp_fractional_scale_v1` scales are in 120ths.
pub const DENOMINATOR: u32 = 120;

/// The scale a surface is drawn at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    /// An integer factor, never 0.
    Integer(u32),
    /// `wp_fractional_scale_v1`, in 120ths, never 0: a buffer under a
    /// viewport.
    Fractional(u32),
}

impl Scale {
    /// The buffer's size in pixels for a surface of `size` logical pixels
    /// at this scale. `None` if a side overflows `u32`; the buffer type
    /// then refuses anything past `i32::MAX` bytes anyway.
    pub fn buffer(self, size: Size) -> Option<(u32, u32)> {
        match self {
            Self::Integer(factor) => {
                let factor = factor.max(1);
                Some((
                    size.width.checked_mul(factor)?,
                    size.height.checked_mul(factor)?,
                ))
            }
            Self::Fractional(v120) => Some((
                scaled_length(size.width, v120)?,
                scaled_length(size.height, v120)?,
            )),
        }
    }

    /// The integer factor for `wl_surface.set_buffer_scale`, where there is
    /// no viewport to size the buffer: a fraction is never sent that way
    /// (the viewport path is the only one that draws at a fraction).
    pub fn integer(self) -> u32 {
        match self {
            Self::Integer(factor) => factor.max(1),
            Self::Fractional(v120) => v120.div_ceil(DENOMINATOR).max(1),
        }
    }

    /// The logical size of an output whose mode, rotated as the output is,
    /// is `device` pixels, as a compositor would work it out: only for a
    /// compositor that leaves the bar's width to scootbar (a `configure`
    /// width of 0). Compositors round differently, so this can be a pixel
    /// off either way. An integer divides and rounds down; a fraction
    /// rounds up. Never 0 on a side that is not 0.
    pub fn logical(self, device: Size) -> Size {
        let side = |length: u32| -> u32 {
            match self {
                Self::Integer(factor) => (length / factor.max(1)).max(length.min(1)),
                Self::Fractional(v120) => {
                    let v120 = u64::from(v120.max(1));
                    let up = (u64::from(length) * u64::from(DENOMINATOR)).div_ceil(v120);
                    // Exceeds `u32` only below scale 1/120, which is not a
                    // scale.
                    u32::try_from(up).unwrap_or(u32::MAX)
                }
            }
        };
        Size {
            width: side(device.width),
            height: side(device.height),
        }
    }

    /// The scale as a number, for messages.
    pub fn value(self) -> f64 {
        match self {
            Self::Integer(factor) => f64::from(factor),
            Self::Fractional(v120) => f64::from(v120) / f64::from(DENOMINATOR),
        }
    }
}

impl fmt::Display for Scale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value())
    }
}

/// `length` logical pixels at `v120`/120, in buffer pixels, rounded halfway
/// away from zero as `wp_fractional_scale_v1` specifies, and at least 1: a
/// buffer side of 0 is no buffer. `None` past `u32`.
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
    /// scootbar makes only with a viewporter to use it with.
    pub fractional: Option<u32>,
    /// `wl_surface.preferred_buffer_scale` (v6), never 0.
    pub buffer_scale: Option<u32>,
}

impl Preferred {
    /// The best scale known: the fractional one, else the larger of the
    /// surface's integer one and the output's (`wl_output.scale`). A
    /// fraction whose rounding up is below `wl_output.scale` is taken as
    /// stale and the integer used (see the module docs).
    pub fn scale(&self, output_scale: u32) -> Scale {
        match self.fractional {
            Some(v120) if v120.div_ceil(DENOMINATOR) >= output_scale.max(1) => {
                Scale::Fractional(v120)
            }
            _ => Scale::Integer(self.buffer_scale.unwrap_or(1).max(output_scale.max(1))),
        }
    }

    /// `preferred_scale`: a scale of 0 is a broken compositor, and ignored
    /// rather than let it reach a size.
    pub fn set_fractional(&mut self, v120: u32) {
        if v120 != 0 {
            self.fractional = Some(v120);
        }
    }

    /// `preferred_buffer_scale`: below 1 is ignored, as for
    /// `wl_output.scale`.
    pub fn set_buffer_scale(&mut self, factor: i32) {
        if let Ok(factor @ 1..) = u32::try_from(factor) {
            self.buffer_scale = Some(factor);
        }
    }
}
