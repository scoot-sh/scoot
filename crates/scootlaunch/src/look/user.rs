//! The user layer: the `[effects]` table of `launch.toml`, applied over
//! the look the layer resolved.
//!
//! Precedence per key: the user's file, then the look layer (the active
//! look and its `[apps.scootlaunch]` overrides), then the built-in
//! default. A table that sets only `motion` leaves `shadow` and `fill` to
//! the look. The table uses the look's own token names and goes through
//! the same parser as the layer (`allowlist::check_app_value`, then the
//! typed slot), so a value is accepted here exactly when a look could set
//! it. `[effects]` reaches only the five depth and motion tokens, never
//! `font-size`, `opacity` or `radius`.
//!
//! A bad value is a note and the key keeps what the look gave it; it
//! never refuses the open. The key names were checked when the file
//! parsed (`Config::check`), so a typo refuses there instead.

use std::path::Path;

use super::{Look, apply, look_dotless};
use crate::config::Config;

/// Applies the config's `[effects]` over `look`, one note per refused
/// value (prefixed with `path`, the user's file). Runs at every open, so
/// an edit shows on the next one.
pub fn apply_user(look: &mut Look, config: &Config, path: &Path, notes: &mut Vec<String>) {
    for (key, value) in &config.effects {
        let mut refused = Vec::new();
        if scoot_look::allowlist::user_effect_key("scootlaunch", "effects", key).is_none() {
            refused.push(format!(
                "`effects.{key}` is not a launcher look key: skipped"
            ));
        } else {
            apply(look_dotless(look, key), "", key, value, &mut refused);
        }
        notes.extend(
            refused
                .into_iter()
                .map(|note| format!("{}: {note}", path.display())),
        );
    }
}

#[cfg(test)]
mod tests;
