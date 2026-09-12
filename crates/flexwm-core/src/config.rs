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
    /// Repairs values that would otherwise be misread silently: unusable
    /// proportions are dropped, an empty list falls back to the defaults, and
    /// the default index and gap are clamped into range.
    pub(crate) fn validated(mut self) -> Self {
        self.column_widths.retain(|p| p.is_finite() && *p > 0.0);
        if self.column_widths.is_empty() {
            self.column_widths = Self::default().column_widths;
        }
        self.default_column_width = self.default_column_width.min(self.column_widths.len() - 1);
        self.gap = self.gap.max(0);
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
    fn validation_keeps_sensible_values() {
        let config = Config {
            gap: 8,
            column_widths: vec![0.5, 1.0],
            default_column_width: 1,
        };
        assert_eq!(config.clone().validated(), config);
    }
}
