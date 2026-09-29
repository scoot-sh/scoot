//! A bar color: `#rrggbb`, opaque.
//!
//! Parsing is strict, as scootbg's is: a `#` and exactly six hex digits,
//! either case, and nothing else (no surrounding whitespace, no `#rgb`
//! shorthand). No alpha yet: a translucent bar is
//! `docs/scootbar/backlog/appearance.md`'s, with the ARGB buffer and the
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
    pub fn xrgb8888(self) -> u32 {
        0xff00_0000 | u32::from(self.r) << 16 | u32::from(self.g) << 8 | u32::from(self.b)
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
