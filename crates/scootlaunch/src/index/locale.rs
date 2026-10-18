//! Localized keys: Desktop Entry Specification §5.
//!
//! The locale comes from `LC_ALL`, then `LC_MESSAGES`, then `LANG`.
//! `.ENCODING` is stripped, and the lookup order is the spec's table:
//! `lang_COUNTRY@MODIFIER`, `lang_COUNTRY`, `lang@MODIFIER`, `lang`,
//! then the default. A modifier or country the locale lacks is never
//! matched.
//!
//! This file is `std`-only.

#[cfg(test)]
mod tests;

/// The locale broken into the pieces the lookup table needs. Parsed
/// from a raw `language[_territory][.codeset][@modifier]` value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locale {
    pub lang: String,
    pub country: Option<String>,
    pub modifier: Option<String>,
}

/// Parses a raw locale value (`de_DE.UTF-8@euro`). Empty values,
/// and the `C`/`POSIX` locales with or without a codeset, parse to
/// no locale (the default key is used on its own).
pub fn parse_locale(raw: &str) -> Option<Locale> {
    if raw.is_empty() {
        return None;
    }
    // The modifier rides after `@`, wherever the codeset sits.
    let (before_modifier, modifier) = match raw.split_once('@') {
        Some((head, modifier)) if !modifier.is_empty() => (head, Some(modifier.to_string())),
        _ => (raw.split_once('@').map_or(raw, |(head, _)| head), None),
    };
    // The codeset rides after `.`, before any `@` (already cut).
    let lang_country = before_modifier
        .split_once('.')
        .map_or(before_modifier, |(head, _)| head);
    let (lang, country) = match lang_country.split_once('_') {
        Some((lang, country)) if !lang.is_empty() && !country.is_empty() => {
            (lang.to_string(), Some(country.to_string()))
        }
        _ => {
            if lang_country.is_empty() {
                return None;
            }
            (lang_country.to_string(), None)
        }
    };
    if lang == "C" || lang == "POSIX" {
        // `C` and `POSIX` carry no language, with or without a
        // codeset: only the default key. (`Name[C]` exists nowhere in
        // the wild; matching it would mint entries no translation
        // ever intended.)
        return None;
    }
    Some(Locale {
        lang,
        country,
        modifier,
    })
}

/// The spec's lookup order for one key: `lang_COUNTRY@MODIFIER`,
/// `lang_COUNTRY`, `lang@MODIFIER`, `lang`. A modifier or country the
/// locale lacks is never matched. The default (unsuffixed) key is not
/// in this list; the caller falls back to it after these miss.
pub fn candidates(locale: &Locale) -> Vec<String> {
    let mut out = Vec::with_capacity(4);
    match (&locale.country, &locale.modifier) {
        (Some(country), Some(modifier)) => {
            out.push(format!("{}_{country}@{modifier}", locale.lang));
            out.push(format!("{}_{country}", locale.lang));
            out.push(format!("{}@{modifier}", locale.lang));
        }
        (Some(country), None) => {
            out.push(format!("{}_{country}", locale.lang));
        }
        // A modifier without a country: `lang@MODIFIER` is still in
        // the table, and only it; nothing with a country is matched.
        (None, Some(modifier)) => {
            out.push(format!("{}@{modifier}", locale.lang));
        }
        (None, None) => {}
    }
    out.push(locale.lang.clone());
    out
}

/// Reads a localized key from a group: the spec's order, then the
/// default. `raw_keys` maps full key names (`Name[de_DE]`) to raw
/// values; escapes apply to whatever is found.
pub fn get_localized<'group>(
    lookup: impl Fn(&str) -> Option<&'group str>,
    key: &str,
    locale: Option<&Locale>,
) -> Option<String> {
    if let Some(locale) = locale {
        for candidate in candidates(locale) {
            let suffixed = format!("{key}[{candidate}]");
            if let Some(raw) = lookup(&suffixed)
                && let Ok(value) = super::parse::unescape(raw)
            {
                return Some(value);
            }
            // A translation that exists but refuses to unescape is a
            // broken key, not a missing one: keep looking (a less
            // specific translation, then the default).
        }
    }
    lookup(key).and_then(|raw| super::parse::unescape(raw).ok())
}
