//! Logical pixels to device pixels: how the repaint puts the layout, which
//! is in logical pixels at every scale, onto a buffer drawn at the
//! compositor's scale.
//!
//! The layout, the input region, the hit test and the motion's geometry
//! never see a scale: they are surface-local logical pixels, and the
//! surface's size is the stack's. Only what is drawn is in device pixels,
//! through a [`Dev`], so a card is the same shape at every scale and the
//! pointer lands on what the pixels show.
//!
//! # Rounding
//!
//! A *position* maps as the protocol maps a size (`scaled_length`: half
//! up), so the stack's far edge lands on the buffer's own. A *length*
//! (width, height, radius, thickness) maps the same way on its own, not as
//! the difference of two positions: a card is then the same size wherever
//! it sits, which is what the shadow's corner tables are keyed by and what
//! lets a card painted once into a layer be the card painted in place. The
//! price is that a card's far edge, or the gap to the next card, can differ
//! by one device pixel from the exact edge; the 8 logical pixel gap is 8
//! or 11 at 1.25x to 1.5x, never more than a pixel from the true value.
//!
//! At scale 1 every mapping is the identity, with no float, so the pixels
//! drawn there are exactly the ones the layout alone drew.

use scootui::density::Scale;
use scootui::paint::ShadowSpec;

use super::layout::Rect;

/// A scale and the logical point it measures from.
///
/// The origin matters to a card painted into a layer: its layout is
/// translated to the layer's corner, and a position there maps as
/// `d(x + origin) - d(origin)`, so the layer, composited at `d(origin)`,
/// puts every pixel where the static paint puts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dev {
    scale: Scale,
    ox: i64,
    oy: i64,
}

impl Dev {
    /// Scale 1: every mapping the identity.
    pub const UNIT: Dev = Dev {
        scale: Scale::Integer(1),
        ox: 0,
        oy: 0,
    };

    /// Positions measured from the surface's corner at `scale`.
    pub fn new(scale: Scale) -> Self {
        Self {
            scale,
            ox: 0,
            oy: 0,
        }
    }

    /// The same scale, with logical positions measured from `(ox, oy)`
    /// instead (a layer's corner, or a bottom anchor's shift).
    pub fn from_origin(self, ox: u32, oy: u32) -> Self {
        Self {
            ox: i64::from(ox),
            oy: i64::from(oy),
            ..self
        }
    }

    /// The scale drawn at.
    pub fn scale(self) -> Scale {
        self.scale
    }

    /// Whether this is scale 1 (the identity).
    pub fn is_unit(self) -> bool {
        self.scale == Scale::Integer(1)
    }

    /// A logical horizontal position as a device position.
    pub fn x(self, x: i64) -> i64 {
        self.scale
            .to_device(x.saturating_add(self.ox))
            .saturating_sub(self.scale.to_device(self.ox))
    }

    /// A logical vertical position as a device position.
    pub fn y(self, y: i64) -> i64 {
        self.scale
            .to_device(y.saturating_add(self.oy))
            .saturating_sub(self.scale.to_device(self.oy))
    }

    /// A logical length as a device length.
    pub fn len(self, length: u32) -> u32 {
        u32::try_from(self.scale.to_device(i64::from(length)).max(0)).unwrap_or(u32::MAX)
    }

    /// A length that must stay visible (a rule, a border, an outline): at
    /// least one device pixel unless it is zero.
    pub fn thick(self, length: u32) -> u32 {
        if length == 0 {
            0
        } else {
            self.len(length).max(1)
        }
    }

    /// A logical position as an unsigned device one, 0 below the surface.
    pub fn x_at(self, x: u32) -> u32 {
        u32::try_from(self.x(i64::from(x)).max(0)).unwrap_or(u32::MAX)
    }

    /// [`x_at`](Self::x_at) for the vertical axis.
    pub fn y_at(self, y: u32) -> u32 {
        u32::try_from(self.y(i64::from(y)).max(0)).unwrap_or(u32::MAX)
    }

    /// A logical rectangle as a device one: where it begins, and how long
    /// each side is (see the module docs). A side that is not zero stays at
    /// least one pixel.
    pub fn rect(self, rect: Rect) -> Rect {
        Rect {
            x: self.x_at(rect.x),
            y: self.y_at(rect.y),
            width: self.thick(rect.width),
            height: self.thick(rect.height),
        }
    }

    /// A shadow's spec at this scale: the blur and the offsets are
    /// lengths (the strength is not).
    pub fn shadow(self, spec: ShadowSpec) -> ShadowSpec {
        let offset = |logical: i32| {
            i32::try_from(self.scale.to_device(i64::from(logical))).unwrap_or(logical)
        };
        ShadowSpec {
            offset_x: offset(spec.offset_x),
            offset_y: offset(spec.offset_y),
            blur: self.len(spec.blur),
            alpha: spec.alpha,
        }
    }

    /// A text size, or any float length (a letter's tracking), in logical
    /// pixels as a device one.
    pub fn em(self, em: f32) -> f32 {
        if self.is_unit() {
            em
        } else {
            em * self.scale.factor() as f32
        }
    }
}

#[cfg(test)]
mod tests;
