//! Everything `scootbar daemon` is told, from its flags (a config file
//! later: `docs/scootbar/backlog/config-cli-and-reload.md`).

use std::path::PathBuf;

use crate::bar::Bar;
use crate::layout::Layout;
use crate::modules::Settings;
use crate::render::Style;
use crate::theme::Theme;

/// The em when `--font-size` is not given, in logical pixels.
pub const DEFAULT_FONT_SIZE: u32 = 14;
/// The largest `--font-size`: past it a glyph is too big to cache (it is
/// still drawn), and no bar wants one that tall.
pub const MAX_FONT_SIZE: u32 = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub bar: Bar,
    pub theme: Theme,
    pub layout: Layout,
    /// `--font`; `None` looks in the well-known places (`crate::font`).
    pub font: Option<PathBuf>,
    /// The em, in logical pixels, 1 to [`MAX_FONT_SIZE`].
    pub font_size: u32,
    pub modules: Settings,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bar: Bar::default(),
            theme: Theme::default(),
            layout: Layout::default(),
            font: None,
            font_size: DEFAULT_FONT_SIZE,
            modules: Settings::default(),
        }
    }
}

impl Config {
    pub fn style(&self) -> Style {
        Style {
            theme: self.theme,
            font_size: self.font_size,
            padding: self.layout.padding,
            spacing: self.layout.spacing,
        }
    }
}
