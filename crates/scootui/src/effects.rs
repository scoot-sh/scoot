//! The look's depth tokens as the shell apps read them: the shadow under a
//! surface (`shadow`) and how its fill is laid down (`fill`).
//!
//! One definition for every app that reads them (`scootnotify`'s cards,
//! `scootlaunch`'s panel), so a value means the same shadow everywhere and
//! the numbers behind `soft`, `deep` and `glow` live in one place. Sizes are
//! logical pixels; an app that draws at another scale scales the spec.

use crate::color::Color;
use crate::paint::ShadowSpec;

/// The depth under a surface (`shadow`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShadowStyle {
    /// No shadow: the default, and free (nothing is built or painted).
    #[default]
    None,
    /// A tight, low shadow: lifts a surface off the window behind it.
    Soft,
    /// A wide, heavy shadow: a surface floating well above the desktop.
    Deep,
    /// A zero-offset shadow tinted with the surface's accent, so it glows.
    Glow,
}

impl ShadowStyle {
    /// The style a `shadow` value names, if it names one.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "none" => Some(Self::None),
            "soft" => Some(Self::Soft),
            "deep" => Some(Self::Deep),
            "glow" => Some(Self::Glow),
            _ => None,
        }
    }

    /// The geometry and strength of the style's shadow, in logical
    /// pixels; `None` for [`ShadowStyle::None`].
    pub fn spec(self) -> Option<ShadowSpec> {
        let (offset_y, blur, alpha) = match self {
            Self::None => return None,
            Self::Soft => (4, 12, 92),
            Self::Deep => (10, 28, 140),
            Self::Glow => (0, 16, 130),
        };
        Some(ShadowSpec {
            offset_x: 0,
            offset_y,
            blur,
            alpha,
        })
    }
}

/// How a surface's fill color is laid down (`fill`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillStyle {
    /// The `surface` color, flat: the default.
    #[default]
    Solid,
    /// A vertical gradient with the light at the top (see
    /// [`FillStyle::stops`]).
    Gradient,
}

impl FillStyle {
    /// The style a `fill` value names, if it names one.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "solid" => Some(Self::Solid),
            "gradient" => Some(Self::Gradient),
            _ => None,
        }
    }

    /// The fill's top and bottom colors over `surface`. Solid: `surface`
    /// twice. A gradient puts the light at the top whatever the look's
    /// polarity: on a dark surface the top edge is lightened a tenth of the
    /// way toward white and the bottom is `surface`; on a light one the top
    /// is `surface` and the bottom is darkened by a twelfth. The shift is
    /// small enough that the text keeps its contrast across the surface.
    pub fn stops(self, surface: Color) -> (Color, Color) {
        const WHITE: Color = Color {
            r: 255,
            g: 255,
            b: 255,
        };
        const BLACK: Color = Color { r: 0, g: 0, b: 0 };
        match self {
            Self::Solid => (surface, surface),
            Self::Gradient if surface.luminance() < 0.5 => (surface.mix(WHITE, 26), surface),
            Self::Gradient => (surface, surface.mix(BLACK, 20)),
        }
    }
}

#[cfg(test)]
mod tests;
