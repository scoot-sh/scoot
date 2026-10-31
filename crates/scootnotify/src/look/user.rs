//! The user layer: the `[effects]`, `[card]` and `[title]` tables of
//! `notify.toml`, applied over the look the layer resolved.
//!
//! Precedence per key: the user's file, then the look layer (the active
//! look and its `[apps.scootnotify]` overrides), then the built-in
//! default. A table that sets only `motion` leaves every other key to the
//! look. The tables use the look's own token names and go through the
//! same parser as the layer (`allowlist::check_app_value`, then the typed
//! slot), so a value is accepted here exactly when a look could set it.
//! `[effects]` reaches only the five depth and motion tokens, never
//! `font-size`, `opacity` or `radius`, and `[colors]` and `[icons]` have
//! no user table.
//!
//! A bad value is a note and the key keeps what the look gave it; it
//! never refuses the file. The key names were checked when the file
//! parsed (`Config::check`), so a typo refuses there instead.

use std::path::Path;

use super::{Look, apply, look_card, look_dotless, look_title};
use crate::config::Config;

/// Applies the config's user tables over `look`, one note per refused
/// value (prefixed with `path`, the user's file). Runs after every
/// resolution of the look (start, reload, look swap), whether or not the
/// look layer is on: these are the user's own values, over the built-ins.
pub fn apply_user(look: &mut Look, config: &Config, path: &Path, notes: &mut Vec<String>) {
    for (table, values) in config.user_look_tables() {
        for (key, value) in values {
            let slot = match table {
                "effects" => look_dotless(look, key),
                "card" => look_card(look, key),
                _ => look_title(look, key),
            };
            // `[effects]` keys are dotless in the look; `[card]` and
            // `[title]` keys carry their table as the dotted prefix.
            let section = if table == "effects" { "" } else { table };
            let mut refused = Vec::new();
            if scoot_look::allowlist::user_effect_key("scootnotify", table, key).is_none() {
                refused.push(format!(
                    "`{table}.{key}` is not a notifier look key: skipped"
                ));
            } else {
                apply(slot, section, key, value, &mut refused);
            }
            notes.extend(
                refused
                    .into_iter()
                    .map(|note| format!("{}: {note}", path.display())),
            );
        }
    }
}

#[cfg(test)]
mod tests;
