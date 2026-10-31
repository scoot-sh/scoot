//! Card styles and typography: the accent marker, the pill outline, the
//! per-app accent color, and the summary's weight and spacing.
//!
//! Sizes are logical pixels, like every other length in the stack. The popup
//! draws them at the compositor's scale (`view::dev`): a 3 px stripe is 3
//! device pixels at 1x and 6 at 2x.
//!
//! Everything here is read once with the look and applied at layout, never
//! per frame. A look that sets none of it paints what it always did;
//! the one layout change that reaches every look is that a bar or buttons
//! now start below a taller icon slot.

use scootui::color::Color;

/// The pill's corner radius, in logical pixels, before the card cuts it
/// back to half its own height (`effective_radius`). A card at most
/// twice this tall is a true pill; a taller one keeps these round ends
/// rather than growing a corner so wide the text runs out of the shape.
pub const PILL_RADIUS: u32 = 36;
/// The extra room, in logical pixels, a pill keeps between its curved
/// ends and the content (text, progress bar, buttons) on each side.
pub const PILL_INSET: u32 = 8;
/// The default summary weight: bold, drawn the way it always was.
pub const DEFAULT_TITLE_WEIGHT: u16 = 700;
/// The accent marker's stripe, in logical pixels.
pub const STRIPE_PX: u32 = 3;
/// The dot's diameter, in logical pixels.
pub const DOT_PX: u32 = 8;
/// The gap between the dot and the app name, in logical pixels.
pub const DOT_GAP: u32 = 6;
/// The contrast text on an app accent reaches (WCAG AA, normal text).
pub const ACCENT_CONTRAST: f64 = 4.5;

/// The accent marker on a card (`card.mark`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CardMark {
    /// No marker: the default.
    #[default]
    None,
    /// A [`STRIPE_PX`] bar down the leading edge, inside the border.
    Stripe,
    /// A dot before the app name on the meta line.
    Dot,
    /// The icon slot on a tinted tile; a card with no icon gets the
    /// app's initial there.
    Badge,
}

impl CardMark {
    /// The marker a `card.mark` value names, if it names one.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "none" => Some(Self::None),
            "stripe" => Some(Self::Stripe),
            "dot" => Some(Self::Dot),
            "badge" => Some(Self::Badge),
            _ => None,
        }
    }
}

/// The card's outline (`card.shape`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CardShape {
    /// The config file's `radius`, else the look's: the default.
    #[default]
    Rounded,
    /// Ends rounded to half the card's height ([`PILL_RADIUS`] at most).
    Pill,
}

impl CardShape {
    /// The shape a `card.shape` value names, if it names one.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "rounded" => Some(Self::Rounded),
            "pill" => Some(Self::Pill),
            _ => None,
        }
    }
}

/// How many times the summary's bold runs are struck, 1 px apart, when the
/// text has no bold face: `title.weight` 100 to 500 is one (regular), 501 to
/// 700 two (bold: what a summary always was), 701 to 900 three. With a bold
/// face the face is the second strike: 501 to 700 draw the bold face once,
/// 701 to 900 draw it twice, 1 px apart.
pub fn title_strikes(weight: u16) -> u8 {
    match weight {
        0..=500 => 1,
        501..=700 => 2,
        _ => 3,
    }
}

/// The mid-tone hue an app's cards carry, from its name: stable across
/// runs and machines (a fixed hash, not a seeded one), case-insensitive in
/// ASCII, and moved toward black or white until `ink` drawn on it reaches
/// [`ACCENT_CONTRAST`]. `None` for a name with nothing in it, which has no
/// identity to color.
///
/// The ink picks the direction: light ink darkens the color, dark ink
/// lightens it. Either direction reaches the target before it runs out
/// (black or white against any ink is at least the square root of 21,
/// about 4.58, away), so the search always ends on a color that passes.
pub fn app_accent(app: &str, ink: Color) -> Option<Color> {
    if app.trim().is_empty() {
        return None;
    }
    let hue = (name_hash(app) % 360) as f32;
    let base = hsl(hue, 0.55, 0.50);
    let toward = if ink.contrast_ratio(BLACK) > ink.contrast_ratio(WHITE) {
        BLACK
    } else {
        WHITE
    };
    let steps = (0..=255u8).step_by(8).chain(std::iter::once(255));
    steps
        .map(|t| base.mix(toward, t))
        .find(|color| ink.contrast_ratio(*color) >= ACCENT_CONTRAST)
}

const BLACK: Color = Color { r: 0, g: 0, b: 0 };
const WHITE: Color = Color {
    r: 255,
    g: 255,
    b: 255,
};

/// FNV-1a over the name's ASCII-lowercased bytes, then the splitmix64
/// finalizer: FNV alone leaves short, similar names (`app1`, `app2`) in
/// neighboring low bits, and the hue is read from those.
fn name_hash(name: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.trim().bytes() {
        hash ^= u64::from(byte.to_ascii_lowercase());
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash ^= hash >> 30;
    hash = hash.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    hash ^= hash >> 27;
    hash = hash.wrapping_mul(0x94d0_49bb_1331_11eb);
    hash ^ (hash >> 31)
}

/// HSL (`hue` in degrees, 0 to 360; `s` and `l` 0 to 1) as a color.
/// Plain `f32` arithmetic: the same bits on every platform.
fn hsl(hue: f32, s: f32, l: f32) -> Color {
    let chroma = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let sector = hue / 60.0;
    let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let (r, g, b) = match sector as u32 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    let offset = l - chroma / 2.0;
    let byte = |c: f32| ((c + offset).clamp(0.0, 1.0) * 255.0).round() as u8;
    Color {
        r: byte(r),
        g: byte(g),
        b: byte(b),
    }
}

#[cfg(test)]
mod tests;
