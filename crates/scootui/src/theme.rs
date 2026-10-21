//! Semantic color tokens, and how a module's state [`Class`] maps onto
//! them. A module names a class, never a color, so a theme source (the
//! config file's `[colors]`, a Stylix one) stays in control of every color
//! the bar draws.
//!
//! `--background` and `--foreground` are flags too; the other tokens are
//! the file's alone. The defaults are Catppuccin Mocha's base, text,
//! yellow, overlay and red.

use crate::class::Class;
use crate::color::Color;

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
    /// A module with a binding, while the pointer is over it (and the
    /// workspaces pill then): the `accent` value unless the config sets
    /// its own, so the tint is the old one by default.
    pub hover: Color,
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
            hover: Color {
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

    /// How a match highlight reads on the selected row (the `accent`
    /// fill): `hover` ink is only legible on some looks (ginger-night's
    /// `#FFB14B` on `#E57F29` is 1.6:1), so the row resolves its own ink
    /// from the measured contrast, always at least 3:1:
    ///
    /// - `hover` itself when it contrasts with the fill (a readable
    ///   `--match-color` or look keeps meaning what its name says on
    ///   both rows);
    /// - else the inverted chip (the row's own text ink as the chip,
    ///   `accent` as its text: the same pair the selected row already
    ///   draws, whose ratio the look pipeline gates);
    /// - else plain black or white ink, whichever contrasts (one always
    ///   reaches 4.6:1, so a hostile look degrades to legible, never to
    ///   a smear).
    pub fn match_on_selection(&self) -> SelectedMatch {
        const MIN: f64 = 3.0;
        if self.hover.contrast_ratio(self.accent) >= MIN {
            return SelectedMatch::Ink(self.hover);
        }
        if self.background.contrast_ratio(self.accent) >= MIN {
            return SelectedMatch::Inverted;
        }
        const BLACK: Color = Color { r: 0, g: 0, b: 0 };
        const WHITE: Color = Color {
            r: 255,
            g: 255,
            b: 255,
        };
        if WHITE.contrast_ratio(self.accent) >= BLACK.contrast_ratio(self.accent) {
            SelectedMatch::Ink(WHITE)
        } else {
            SelectedMatch::Ink(BLACK)
        }
    }
}

/// How [`Theme::match_on_selection`] draws one matched span on the
/// selected row: `Ink` overdraws the span in that color on the `accent`
/// fill; `Inverted` fills the span's box with the row's text ink
/// (`background`) and draws the span in `accent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedMatch {
    Ink(Color),
    Inverted,
}
