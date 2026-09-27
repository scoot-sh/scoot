//! A solid wallpaper color: `#rrggbb`, opaque.
//!
//! Parsing is strict: a `#` and exactly six hex digits, either case, and
//! nothing else (no surrounding whitespace, no `#rgb` shorthand, no alpha).
//! A wallpaper is the bottom of the stack, so there is nothing under it to
//! blend with: a color is always opaque.

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
pub enum ColorError {
    /// It does not start with `#`: in `scootbg set`, that is a path, and
    /// images arrive in a later version.
    NotAColor,
    /// It starts with `#` but is not `#` and six hex digits.
    Malformed,
}

impl fmt::Display for ColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAColor => write!(
                f,
                "only colors can be set for now (\"#rrggbb\"); images come in a later version"
            ),
            Self::Malformed => write!(
                f,
                "a color is '#' and six hex digits, \"#rrggbb\" (such as \"#1e1e2e\")"
            ),
        }
    }
}

impl Color {
    /// Parses `#rrggbb` (hex digits in either case), strictly.
    pub fn parse(text: &str) -> Result<Self, ColorError> {
        let Some(hex) = text.strip_prefix('#') else {
            return Err(ColorError::NotAColor);
        };
        let [r1, r2, g1, g2, b1, b2] = *hex.as_bytes() else {
            return Err(ColorError::Malformed);
        };
        let byte = |high: u8, low: u8| Some(nibble(high)? << 4 | nibble(low)?);
        match (byte(r1, r2), byte(g1, g2), byte(b1, b2)) {
            (Some(r), Some(g), Some(b)) => Ok(Self { r, g, b }),
            _ => Err(ColorError::Malformed),
        }
    }

    /// The four `u32` channels `wp_single_pixel_buffer_manager_v1` takes,
    /// premultiplied (a no-op: alpha is full). Each 8-bit value `v` becomes
    /// `v * 0x01010101`, which is exactly `v / 255` of `u32::MAX`: 0 maps to
    /// 0, 255 to `u32::MAX`, and every step is equal, so the compositor's
    /// conversion back to 8 bits returns `v` unchanged.
    pub fn single_pixel(self) -> [u32; 4] {
        let wide = |v: u8| u32::from(v) * 0x0101_0101;
        [wide(self.r), wide(self.g), wide(self.b), u32::MAX]
    }

    /// One `XRGB8888` pixel as it sits in memory: `wl_shm` formats are
    /// little-endian, so blue, green, red, then the unused byte, set to
    /// 0xff so the buffer reads as opaque even to code that looks at it.
    pub fn xrgb8888(self) -> [u8; 4] {
        [self.b, self.g, self.r, 0xff]
    }
}

/// `#rrggbb`, lowercase: the form `query` reports.
impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// As its `#rrggbb` string, written straight into the output (no
/// intermediate `String`).
impl serde::Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
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
