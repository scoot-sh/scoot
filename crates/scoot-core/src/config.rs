//! Tunables that shape layout.

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// Gap between columns, between windows in a column, and at output edges.
    pub gap: i32,
    /// Column widths as proportions of the output, in the order
    /// [`Action::CycleColumnWidth`](crate::Action::CycleColumnWidth) steps
    /// through them.
    pub column_widths: Vec<f64>,
    /// Index into `column_widths` for new columns.
    pub default_column_width: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            gap: 12,
            column_widths: vec![1.0 / 3.0, 0.5, 2.0 / 3.0],
            default_column_width: 1,
        }
    }
}

impl Config {
    /// The largest [`gap`](Self::gap) a config may ask for.
    ///
    /// Two reasons for this exact number, both about `i32` arithmetic rather
    /// than taste:
    ///
    /// - **Nothing real can use more.** 10,000 px is past the long edge of an
    ///   8K display (7680), so a gap this size already insets every real
    ///   output to a zero-sized usable area -- there is no configuration a
    ///   user could want beyond it, and one asking for it is a typo or a
    ///   probe, not a preference.
    /// - **It keeps every gap-derived expression in the layout far from
    ///   overflow** for any size a real display reports: `2 * by` in
    ///   [`Rect::inset`](crate::Rect::inset), `available + gap` in
    ///   `layout::column_width`, and `height + gap` in `World::arrange`. An
    ///   unbounded gap overflows the first of those on its own -- `2 *
    ///   i32::MAX` is a panic in a debug build and a wrapped, negative inset
    ///   in release -- which is what this closes.
    ///
    /// It deliberately does *not* claim to bound `gap * (windows - 1)` in
    /// `World::column_heights`: that product is bounded by how many windows
    /// one column holds, not by this cap, and a cap low enough to bound it
    /// for an arbitrary window count would be too low to be a gap.
    pub const MAX_GAP: i32 = 10_000;

    /// Brings a configured gap into the range the layout's arithmetic is
    /// safe for -- see [`Self::MAX_GAP`].
    ///
    /// Public because the gap is clamped in two places that must agree:
    /// here, and `scoot`'s own config loader, which sizes the focus ring
    /// against the gap *before* a [`World`](crate::World) exists to validate
    /// it.
    pub fn clamp_gap(gap: i32) -> i32 {
        gap.clamp(0, Self::MAX_GAP)
    }

    /// Repairs values that would otherwise be misread silently: unusable
    /// proportions are dropped, an empty list falls back to the defaults, and
    /// the default index and gap are clamped into range.
    pub(crate) fn validated(mut self) -> Self {
        self.column_widths.retain(|p| p.is_finite() && *p > 0.0);
        if self.column_widths.is_empty() {
            self.column_widths = Self::default().column_widths;
        }
        self.default_column_width = self.default_column_width.min(self.column_widths.len() - 1);
        self.gap = Self::clamp_gap(self.gap);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_repairs_unusable_values() {
        let config = Config {
            gap: -4,
            column_widths: vec![0.0, f64::NAN],
            default_column_width: 9,
        }
        .validated();
        assert_eq!(config.gap, 0);
        assert_eq!(config.column_widths, Config::default().column_widths);
        assert_eq!(config.default_column_width, 2);
    }

    #[test]
    fn validation_caps_an_enormous_gap() {
        // One under, exactly at, one over, and the extreme a config can spell.
        for (configured, expected) in [
            (Config::MAX_GAP - 1, Config::MAX_GAP - 1),
            (Config::MAX_GAP, Config::MAX_GAP),
            (Config::MAX_GAP + 1, Config::MAX_GAP),
            (i32::MAX, Config::MAX_GAP),
        ] {
            let config = Config {
                gap: configured,
                ..Config::default()
            }
            .validated();
            assert_eq!(config.gap, expected, "gap {configured} clamped wrongly");
        }
    }

    /// The reason [`Config::MAX_GAP`] exists: an unbounded gap overflows
    /// `2 * by` inside `Rect::inset`, which every arrangement runs. This is
    /// the smallest expression that proves the cap is what keeps that add in
    /// range -- it fails (debug: overflow panic) without the clamp.
    #[test]
    fn the_capped_gap_cannot_overflow_an_inset() {
        let gap = Config {
            gap: i32::MAX,
            ..Config::default()
        }
        .validated()
        .gap;
        let usable = crate::Rect::new(0, 0, 1600, 1000).inset(gap);
        assert_eq!(usable.size(), crate::Size::new(0, 0));
    }

    #[test]
    fn validation_keeps_sensible_values() {
        let config = Config {
            gap: 8,
            column_widths: vec![0.5, 1.0],
            default_column_width: 1,
        };
        assert_eq!(config.clone().validated(), config);
    }
}
