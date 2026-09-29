//! Semantic color tokens, and how a module's state [`Class`] maps onto
//! them. A module names a class, never a color, so a theme source (the
//! config file's `[colors]`, a Stylix one) stays in control of every color
//! the bar draws.
//!
//! `--background` and `--foreground` are flags too; the other tokens are
//! the file's alone. The defaults are Catppuccin Mocha's base, text,
//! yellow, overlay and red.

use crate::color::Color;
use crate::modules::Class;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// The bar.
    pub background: Color,
    /// Text in the `normal` class.
    pub foreground: Color,
    /// The `warn` class, and a highlight (the active workspace, later).
    pub accent: Color,
    /// The `muted` class.
    pub dim: Color,
    /// The `urgent` class.
    pub urgent: Color,
}

pub const DEFAULT_BACKGROUND: Color = Color {
    r: 0x1e,
    g: 0x1e,
    b: 0x2e,
};
pub const DEFAULT_FOREGROUND: Color = Color {
    r: 0xcd,
    g: 0xd6,
    b: 0xf4,
};

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: DEFAULT_BACKGROUND,
            foreground: DEFAULT_FOREGROUND,
            accent: Color {
                r: 0xf9,
                g: 0xe2,
                b: 0xaf,
            },
            dim: Color {
                r: 0x6c,
                g: 0x70,
                b: 0x86,
            },
            urgent: Color {
                r: 0xf3,
                g: 0x8b,
                b: 0xa8,
            },
        }
    }
}

impl Theme {
    /// The color a view in `class` is drawn in.
    pub fn class(&self, class: Class) -> Color {
        match class {
            Class::Normal => self.foreground,
            Class::Warn => self.accent,
            Class::Urgent => self.urgent,
            Class::Muted => self.dim,
        }
    }
}
