//! What the bar is, independent of any output: its edge, height and
//! margins, from the config file and the flags, and the
//! layer-surface requests they turn into. Pure, so every rule is a unit
//! test.
//!
//! ## The exclusive zone is the bar's height, not height plus margin
//!
//! `zwlr_layer_surface_v1.set_margin` says "the exclusive zone includes the
//! margin": a compositor reserves the zone *plus* the margin on the
//! anchored edge. The pinned Smithay's `LayerMap::arrange`
//! (`src/desktop/wayland/layer.rs` at `035d447`) does exactly that
//! (`zone.loc.y += amount + margin.top` for a top bar), and so scoot does;
//! `tests/bar.rs` pins it with a screenshot. So a top bar with `--margin 8`
//! sets its zone to its height, and windows start `height + 8` pixels from
//! the top, with the 8 pixels above the bar left empty.
//!
//! The zone is set before the surface's first commit, which carries no
//! buffer. scoot applies it from that commit
//! (`docs/backlog/resolved/layer-surface-bufferless-exclusive-zone-done.md`),
//! so windows move out of the bar's way once, before it draws, and do not
//! jump again when its first buffer lands.

use std::fmt;

#[cfg(test)]
mod tests;

/// The bar's height when `--height` is not given, in logical pixels.
pub const DEFAULT_HEIGHT: u32 = 28;
/// The tallest bar `--height` takes. Anything near it is a mistake rather
/// than a bar; the bound keeps every size derived from it far from
/// overflowing an `i32` (the protocol's type) at any scale.
pub const MAX_HEIGHT: u32 = 1024;
/// The largest margin `--margin` takes on a side, for the same reason.
pub const MAX_MARGIN: u32 = 1024;

/// The output edge the bar is anchored to. Vertical bars are not in the
/// first version, on purpose (`docs/scootbar/cli.md`, Edge cases).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Edge {
    #[default]
    Top,
    Bottom,
}

impl Edge {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "top" => Some(Self::Top),
            "bottom" => Some(Self::Bottom),
            _ => None,
        }
    }
}

/// The layer-shell layer the bar sits in. `background` is left out: it
/// is the wallpaper's layer, not a bar's. A fullscreen window hides `top` in scoot
/// (`docs/protocols.md`), and never `overlay`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layer {
    /// Behind windows: they cover the bar wherever they overlap it.
    Bottom,
    /// In front of windows, hidden by a fullscreen one.
    #[default]
    Top,
    /// In front of everything, fullscreen windows included.
    Overlay,
}

impl Layer {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "bottom" => Some(Self::Bottom),
            "top" => Some(Self::Top),
            "overlay" => Some(Self::Overlay),
            _ => None,
        }
    }
}

/// Parses `--exclusive` and `bar.exclusive`'s text: `true` or `false`.
pub fn parse_bool(text: &str) -> Option<bool> {
    match text {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Space between the bar and the output's edges, in logical pixels, sent
/// as the layer surface's own margin (never drawn as transparent pixels:
/// the surface is exactly the bar). A margin on the edge opposite the
/// anchored one does nothing; the protocol says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Margin {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
}

/// Why a `--margin` value is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarginError {
    /// Not one to four comma-separated whole numbers.
    Malformed,
    /// A value above [`MAX_MARGIN`].
    TooLarge,
}

impl fmt::Display for MarginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed => write!(
                f,
                "a margin is one to four whole numbers separated by commas, \
                 as in CSS: ALL, VERTICAL,HORIZONTAL, TOP,HORIZONTAL,BOTTOM \
                 or TOP,RIGHT,BOTTOM,LEFT"
            ),
            Self::TooLarge => write!(f, "a margin is at most {MAX_MARGIN}"),
        }
    }
}

impl Margin {
    /// CSS shorthand, comma-separated so it needs no quoting: `8` (every
    /// side), `8,4` (top and bottom, left and right), `8,4,0` (top, left
    /// and right, bottom) or `8,4,0,4` (top, right, bottom, left).
    pub fn parse(text: &str) -> Result<Self, MarginError> {
        let mut values = [0u32; 4];
        let mut count = 0;
        for part in text.split(',') {
            let slot = values.get_mut(count).ok_or(MarginError::Malformed)?;
            *slot = parse_whole(part).ok_or(MarginError::Malformed)?;
            if *slot > MAX_MARGIN {
                return Err(MarginError::TooLarge);
            }
            count += 1;
        }
        let [a, b, c, d] = values;
        Ok(match count {
            1 => Self {
                top: a,
                right: a,
                bottom: a,
                left: a,
            },
            2 => Self {
                top: a,
                right: b,
                bottom: a,
                left: b,
            },
            3 => Self {
                top: a,
                right: b,
                bottom: c,
                left: b,
            },
            // `split` yields at least one part, and a fifth is refused
            // above, so this is exactly four.
            _ => Self {
                top: a,
                right: b,
                bottom: c,
                left: d,
            },
        })
    }
}

/// Digits only: no sign, no spaces, no `+`, not empty. `u32::from_str`
/// takes a leading `+`, which a margin has no use for.
pub fn parse_whole(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Parses a `--height` value: a whole number from 1 to [`MAX_HEIGHT`].
pub fn parse_height(text: &str) -> Option<u32> {
    parse_whole(text).filter(|height| (1..=MAX_HEIGHT).contains(height))
}

/// The bar, as the command line set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bar {
    pub edge: Edge,
    /// Logical pixels, 1 to [`MAX_HEIGHT`].
    pub height: u32,
    pub margin: Margin,
    pub layer: Layer,
    /// Whether the bar reserves its height, so windows are arranged beside
    /// it; `false` floats it over them (exclusive zone -1).
    pub exclusive: bool,
}

impl Default for Bar {
    fn default() -> Self {
        Self {
            edge: Edge::Top,
            height: DEFAULT_HEIGHT,
            margin: Margin::default(),
            layer: Layer::Top,
            exclusive: true,
        }
    }
}

/// Which edges the layer surface is anchored to: the bar's edge and both
/// sides, so the compositor sizes its width to the output (less the side
/// margins).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchors {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl Bar {
    pub fn anchors(&self) -> Anchors {
        Anchors {
            top: self.edge == Edge::Top,
            bottom: self.edge == Edge::Bottom,
            left: true,
            right: true,
        }
    }

    /// `set_size`: width 0 (the compositor's to choose, as the surface is
    /// anchored to both sides) and the bar's height.
    pub fn requested_size(&self) -> (u32, u32) {
        (0, self.height)
    }

    /// `set_exclusive_zone`: the bar's height, or -1 for a bar that
    /// reserves nothing and ignores the zones of others (0 would still
    /// keep clear of another bar's zone; -1 is the protocol's "float").
    /// The compositor adds the margin on the anchored edge itself (see the
    /// module docs).
    pub fn exclusive_zone(&self) -> i32 {
        if !self.exclusive {
            return -1;
        }
        // At most `MAX_HEIGHT`, so it fits; saturate rather than wrap all
        // the same.
        i32::try_from(self.height).unwrap_or(i32::MAX)
    }

    /// `set_margin`'s arguments, in its order: top, right, bottom, left.
    pub fn margins(&self) -> [i32; 4] {
        let side = |value: u32| i32::try_from(value).unwrap_or(i32::MAX);
        let m = self.margin;
        [side(m.top), side(m.right), side(m.bottom), side(m.left)]
    }

    /// The bar's width on an output `output_width` logical pixels wide, for
    /// a compositor that configures a width of 0 (none checked does, for a
    /// surface anchored to both sides): the output less the side margins,
    /// and never 0.
    pub fn width_on(&self, output_width: u32) -> u32 {
        output_width
            .saturating_sub(self.margin.left)
            .saturating_sub(self.margin.right)
            .max(1)
    }
}
