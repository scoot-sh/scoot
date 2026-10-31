//! `[gestures]`: parsing the table into a [`GestureConfig`].
//!
//! Every key is kept as a raw TOML value so a wrong type or an unknown name
//! warns and falls back to the default for that key alone, instead of failing
//! the file (the same discipline as `[scroll] step`).

use serde::Deserialize;

use super::{
    DEFAULT_FINGERS, DEFAULT_SENSITIVITY, GestureConfig, HorizontalMode, MAX_SENSITIVITY,
    MIN_SENSITIVITY, VerticalMode,
};

/// The finger counts a swipe can be claimed at.
const FINGERS: [u32; 2] = [3, 4];

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(in crate::compositor) struct GesturesFile {
    swipe_fingers: Option<toml::Value>,
    horizontal: Option<toml::Value>,
    vertical: Option<toml::Value>,
    natural: Option<toml::Value>,
    sensitivity: Option<toml::Value>,
}

impl GesturesFile {
    /// The table as the compositor runs it: defaults for what is unset,
    /// a warned default for what is wrong, a warned clamp for what is out of
    /// range.
    pub(in crate::compositor) fn resolve(&self) -> GestureConfig {
        let default = GestureConfig::default();
        GestureConfig {
            fingers: self.fingers().unwrap_or(default.fingers),
            horizontal: choice(
                "horizontal",
                self.horizontal.as_ref(),
                HorizontalMode::parse,
                "\"focus-column\" or \"none\"",
            )
            .unwrap_or(default.horizontal),
            vertical: choice(
                "vertical",
                self.vertical.as_ref(),
                VerticalMode::parse,
                "\"focus-workspace\", \"focus-window\" or \"none\"",
            )
            .unwrap_or(default.vertical),
            natural: self.natural().unwrap_or(default.natural),
            sensitivity: self.sensitivity().unwrap_or(default.sensitivity),
        }
    }

    fn fingers(&self) -> Option<u32> {
        let value = self.swipe_fingers.as_ref()?;
        match value.as_integer().and_then(|n| u32::try_from(n).ok()) {
            Some(n) if FINGERS.contains(&n) => Some(n),
            _ => {
                tracing::warn!(
                    swipe_fingers = %value,
                    "[gestures] swipe_fingers must be 3 or 4; using {DEFAULT_FINGERS}"
                );
                None
            }
        }
    }

    fn natural(&self) -> Option<bool> {
        let value = self.natural.as_ref()?;
        let natural = value.as_bool();
        if natural.is_none() {
            tracing::warn!(natural = %value, "[gestures] natural must be true or false; using true");
        }
        natural
    }

    fn sensitivity(&self) -> Option<f64> {
        let value = self.sensitivity.as_ref()?;
        let number = match value {
            toml::Value::Float(number) => *number,
            toml::Value::Integer(number) => *number as f64,
            other => {
                tracing::warn!(
                    sensitivity = %other,
                    "[gestures] sensitivity must be a number; using {DEFAULT_SENSITIVITY}"
                );
                return None;
            }
        };
        if !number.is_finite() {
            tracing::warn!(
                sensitivity = number,
                "[gestures] sensitivity must be finite; using {DEFAULT_SENSITIVITY}"
            );
            return None;
        }
        let clamped = number.clamp(MIN_SENSITIVITY, MAX_SENSITIVITY);
        if clamped != number {
            tracing::warn!(
                sensitivity = number,
                "[gestures] sensitivity is outside {MIN_SENSITIVITY}..={MAX_SENSITIVITY}; clamped"
            );
        }
        Some(clamped)
    }
}

/// A string key naming one of a closed set of modes.
fn choice<T>(
    key: &str,
    value: Option<&toml::Value>,
    parse: fn(&str) -> Option<T>,
    expected: &str,
) -> Option<T> {
    let value = value?;
    let parsed = value.as_str().and_then(parse);
    if parsed.is_none() {
        tracing::warn!(
            value = %value,
            "[gestures] {key} must be {expected}; using the default"
        );
    }
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(text: &str) -> GestureConfig {
        toml::from_str::<GesturesFile>(text)
            .expect("the table parses")
            .resolve()
    }

    #[test]
    fn unset_is_the_default() {
        assert_eq!(resolve(""), GestureConfig::default());
    }

    #[test]
    fn every_key_is_read() {
        let config = resolve(
            "swipe_fingers = 4\nhorizontal = \"none\"\nvertical = \"focus-window\"\n\
             natural = false\nsensitivity = 2",
        );
        assert_eq!(
            config,
            GestureConfig {
                fingers: 4,
                horizontal: HorizontalMode::None,
                vertical: VerticalMode::FocusWindow,
                natural: false,
                sensitivity: 2.0,
            }
        );
    }

    #[test]
    fn a_bad_value_falls_back_for_that_key_alone() {
        let config = resolve(
            "swipe_fingers = 5\nhorizontal = \"sideways\"\nvertical = 3\nnatural = \"yes\"\n\
             sensitivity = \"fast\"",
        );
        assert_eq!(config, GestureConfig::default());
        let config = resolve("swipe_fingers = 2\nvertical = \"none\"");
        assert_eq!(config.fingers, DEFAULT_FINGERS);
        assert_eq!(config.vertical, VerticalMode::None);
    }

    #[test]
    fn sensitivity_is_clamped_and_never_not_a_number() {
        assert_eq!(resolve("sensitivity = 0.0").sensitivity, MIN_SENSITIVITY);
        assert_eq!(resolve("sensitivity = -3").sensitivity, MIN_SENSITIVITY);
        assert_eq!(resolve("sensitivity = 1e9").sensitivity, MAX_SENSITIVITY);
        assert_eq!(
            resolve("sensitivity = nan").sensitivity,
            DEFAULT_SENSITIVITY
        );
        assert_eq!(
            resolve("sensitivity = inf").sensitivity,
            DEFAULT_SENSITIVITY
        );
        assert_eq!(resolve("sensitivity = 0.5").sensitivity, 0.5);
    }

    #[test]
    fn an_unknown_key_is_refused_like_every_other_table() {
        assert!(toml::from_str::<GesturesFile>("fingers = 3").is_err());
    }
}
