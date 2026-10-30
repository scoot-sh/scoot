//! A bar color: `#rrggbb`, opaque.
//!
//! Parsing is strict, as scootbg's is: a `#` and exactly six hex digits,
//! either case, and nothing else (no surrounding whitespace, no `#rgb`
//! shorthand). No alpha yet: a translucent bar is
//! `docs/scootbar/backlog/resolved/appearance-done.md`'s, with the ARGB buffer and the
//! blend cost it measures.

use std::fmt;

#[cfg(test)]
mod tests;

/// An opaque 8-bit-per-channel color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Why a string is not a color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorError;

impl fmt::Display for ColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a color is '#' and six hex digits, \"#rrggbb\" (such as \"#1e1e2e\")"
        )
    }
}

impl Color {
    /// Parses `#rrggbb` (hex digits in either case), strictly.
    pub fn parse(text: &str) -> Result<Self, ColorError> {
        let hex = text.strip_prefix('#').ok_or(ColorError)?;
        let [r1, r2, g1, g2, b1, b2] = *hex.as_bytes() else {
            return Err(ColorError);
        };
        let byte = |high: u8, low: u8| Some(nibble(high)? << 4 | nibble(low)?);
        match (byte(r1, r2), byte(g1, g2), byte(b1, b2)) {
            (Some(r), Some(g), Some(b)) => Ok(Self { r, g, b }),
            _ => Err(ColorError),
        }
    }

    /// One `XRGB8888` pixel as a native `u32`: `0xffRRGGBB`. `wl_shm`
    /// formats are little-endian, so written with `to_le_bytes` it sits in
    /// memory as blue, green, red, then the unused byte, set to 0xff so the
    /// buffer reads as opaque even to code that looks at it.
    #[allow(dead_code)] // Only `fill_span` (the workspaces module) reads it.
    pub fn xrgb8888(self) -> u32 {
        0xff00_0000 | u32::from(self.r) << 16 | u32::from(self.g) << 8 | u32::from(self.b)
    }

    /// One premultiplied `ARGB8888` pixel of this color at `alpha` (0
    /// transparent, 255 opaque): each channel scaled by the alpha, rounded,
    /// as `wl_shm`'s `argb8888` (and every compositor's blend) expects.
    /// At 255 it is [`Color::xrgb8888`].
    pub fn argb8888(self, alpha: u8) -> u32 {
        let scale = |channel: u8| (u32::from(channel) * u32::from(alpha) + 127) / 255;
        u32::from(alpha) << 24 | scale(self.r) << 16 | scale(self.g) << 8 | scale(self.b)
    }

    /// This color moved `toward` out of 255 of the way to `other`, rounded:
    /// 0 is this color, 255 is `other`. The gradient's stops and the tint
    /// of a lightened edge are built with it.
    pub fn mix(self, other: Color, toward: u8) -> Color {
        let t = u32::from(toward);
        let channel = |from: u8, to: u8| {
            ((u32::from(from) * (255 - t) + u32::from(to) * t + 127) / 255) as u8
        };
        Color {
            r: channel(self.r, other.r),
            g: channel(self.g, other.g),
            b: channel(self.b, other.b),
        }
    }

    /// WCAG relative luminance (0 black, 1 white): the same formula the
    /// look pipeline's contrast check uses, over the parsed channels.
    pub fn luminance(self) -> f64 {
        let channel = |byte: u8| {
            let c = f64::from(byte) / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }

    /// WCAG contrast ratio against `other` (1 identical, 21 black on
    /// white). Pure computation, no allocation.
    pub fn contrast_ratio(self, other: Color) -> f64 {
        let (x, y) = (self.luminance(), other.luminance());
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }
}

/// `#rrggbb`, lowercase.
impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

fn nibble(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}
