//! Where each module goes along the bar: three sections, `left`, `center`
//! and `right`, each a list of module ids, and the spans they get. Pure.
//!
//! - **Left** modules are packed from the left edge, **right** ones from
//!   the right edge, in the order listed; **center** ones are packed
//!   together, centered on the bar.
//! - A module's width is its measured content plus its padding on both
//!   sides; `spacing` separates neighbours in a section. A module that
//!   shows nothing (an empty view, or not ready yet) takes no space at all,
//!   padding and spacing included.
//! - **When they do not fit**, the left section keeps its place, the right
//!   one gives way to it, and the center one is pushed off center to fit
//!   between them, then cut. Every span is clipped to the bar and none
//!   overlaps another, so one module's redraw can never paint over a
//!   neighbour: the bar clips deliberately, never by overflow (every sum
//!   here saturates).

use std::fmt;

use crate::paint::Span;

#[cfg(test)]
mod tests;

/// Default padding on each side of a module, in logical pixels.
pub const DEFAULT_PADDING: u32 = 8;
/// Default space between neighbouring modules, in logical pixels.
pub const DEFAULT_SPACING: u32 = 0;
/// The most `--padding` and `--spacing` take.
pub const MAX_GAP: u32 = 1024;
/// The most modules a layout holds.
pub const MAX_MODULES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Left,
    Center,
    Right,
}

impl Section {
    pub const ALL: [Self; 3] = [Self::Left, Self::Center, Self::Right];

    pub fn flag(self) -> &'static str {
        match self {
            Self::Left => "--left",
            Self::Center => "--center",
            Self::Right => "--right",
        }
    }

    /// The section's name in the config file and the `query` reply:
    /// `left`, `center` or `right`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
        }
    }
}

/// The layout as configured: module ids per section, and the gaps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub left: Vec<&'static str>,
    pub center: Vec<&'static str>,
    pub right: Vec<&'static str>,
    /// Logical pixels on each side of a module's content.
    pub padding: u32,
    /// Logical pixels between neighbouring modules.
    pub spacing: u32,
}

impl Default for Layout {
    /// The clock in the center, when this build has it.
    fn default() -> Self {
        let center = crate::modules::REGISTRY
            .iter()
            .map(|spec| spec.id)
            .filter(|&id| id == "clock")
            .collect();
        Self {
            left: Vec::new(),
            center,
            right: Vec::new(),
            padding: DEFAULT_PADDING,
            spacing: DEFAULT_SPACING,
        }
    }
}

impl Layout {
    pub fn section(&self, section: Section) -> &[&'static str] {
        match section {
            Section::Left => &self.left,
            Section::Center => &self.center,
            Section::Right => &self.right,
        }
    }

    /// Every placed module, left to right by section: `(section, id)`.
    pub fn placed(&self) -> impl Iterator<Item = (Section, &'static str)> + '_ {
        Section::ALL
            .into_iter()
            .flat_map(|section| self.section(section).iter().map(move |&id| (section, id)))
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.left.is_empty() && self.center.is_empty() && self.right.is_empty()
    }
}

/// Why placed modules are refused: a duplicate or too many. Unknown ids
/// are refused earlier, where the list is read (a flag or a file key), so
/// they name that context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementError {
    /// Placed twice (in one section or across two).
    Twice(&'static str),
    TooMany,
}

impl fmt::Display for PlacementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Twice(id) => write!(f, "`{id}` is placed twice; a module goes in one place"),
            Self::TooMany => write!(f, "at most {MAX_MODULES} modules"),
        }
    }
}

/// No module twice across the sections, and at most [`MAX_MODULES`].
/// `Err` is the offending section and why, for the caller to name its own
/// context (a flag or a file key).
pub fn check_placement(layout: &Layout) -> Result<(), (Section, PlacementError)> {
    let mut seen: Vec<&str> = Vec::new();
    for section in Section::ALL {
        for &id in layout.section(section) {
            if seen.contains(&id) {
                return Err((section, PlacementError::Twice(id)));
            }
            if seen.len() >= MAX_MODULES {
                return Err((section, PlacementError::TooMany));
            }
            seen.push(id);
        }
    }
    Ok(())
}

/// Lays out modules `sections[i]` wide `widths[i]` (device pixels, padding
/// included; 0 takes no space) with `spacing` between neighbours on a bar
/// `bar` wide, into `spans`. The three slices are as long as each other;
/// modules of one section must be contiguous and in order, as
/// [`Layout::placed`] lists them.
pub fn arrange(sections: &[Section], widths: &[u32], spacing: u32, bar: u32, spans: &mut [Span]) {
    let bar = i64::from(bar);
    let spacing = i64::from(spacing);
    // Each section's total width, spacing included.
    let total = |wanted: Section| -> i64 {
        let mut sum = 0i64;
        let mut count = 0i64;
        for (section, width) in sections.iter().zip(widths) {
            if *section == wanted && *width > 0 {
                sum = sum.saturating_add(i64::from(*width));
                count += 1;
            }
        }
        sum.saturating_add(spacing.saturating_mul((count - 1).max(0)))
    };
    let left = total(Section::Left);
    let center = total(Section::Center);
    let right = total(Section::Right);
    // Where each section starts, and the room it may use: `[lo, hi)`.
    let left_end = left.min(bar);
    let right_start = (bar - right).max(left_end);
    let center_start = ((bar - center) / 2).min(right_start - center).max(left_end);
    let bounds = |section: Section| match section {
        Section::Left => (0, 0, left_end),
        Section::Center => (center_start, left_end, right_start),
        Section::Right => (right_start, right_start, bar),
    };
    let mut pens = [
        bounds(Section::Left).0,
        bounds(Section::Center).0,
        bounds(Section::Right).0,
    ];
    for ((section, width), span) in sections.iter().zip(widths).zip(spans.iter_mut()) {
        let (_, lo, hi) = bounds(*section);
        let pen = &mut pens[*section as usize];
        if *width == 0 {
            *span = Span {
                x: clamp_u32(*pen, lo, hi),
                width: 0,
            };
            continue;
        }
        let start = *pen;
        let end = start.saturating_add(i64::from(*width));
        *pen = end.saturating_add(spacing);
        let x = start.clamp(lo, hi.max(lo));
        let end = end.clamp(x, hi.max(x));
        *span = Span {
            x: clamp_u32(x, 0, i64::from(u32::MAX)),
            width: clamp_u32(end - x, 0, i64::from(u32::MAX)),
        };
    }
}

fn clamp_u32(value: i64, lo: i64, hi: i64) -> u32 {
    // Clamped into `u32`'s range first, so the cast is exact.
    value.clamp(lo.max(0), hi.clamp(0, i64::from(u32::MAX)).max(lo.max(0))) as u32
}
