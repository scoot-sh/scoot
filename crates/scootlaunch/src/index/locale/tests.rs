//! Tests for localized keys: the spec's table order, in full.

use super::*;
use std::collections::HashMap;

fn fixture(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn get<'map>(map: &'map HashMap<String, String>) -> impl Fn(&str) -> Option<&'map str> {
    move |key: &str| map.get(key).map(String::as_str)
}

#[test]
fn full_locale_tries_the_whole_table_in_order() {
    let locale = parse_locale("de_DE.UTF-8@euro").unwrap();
    assert_eq!(
        candidates(&locale),
        vec!["de_DE@euro", "de_DE", "de@euro", "de"]
    );
}

#[test]
fn locales_without_pieces_match_nothing_they_lack() {
    assert_eq!(
        candidates(&parse_locale("de_DE.UTF-8").unwrap()),
        vec!["de_DE", "de"]
    );
    assert_eq!(
        candidates(&parse_locale("de@euro").unwrap()),
        vec!["de@euro", "de"]
    );
    assert_eq!(candidates(&parse_locale("de").unwrap()), vec!["de"]);
    assert_eq!(candidates(&parse_locale("de.UTF-8").unwrap()), vec!["de"]);
}

#[test]
fn c_and_posix_and_empty_are_no_locale() {
    assert_eq!(parse_locale(""), None);
    assert_eq!(parse_locale("C"), None);
    assert_eq!(parse_locale("C.UTF-8"), None);
    assert_eq!(parse_locale("POSIX"), None);
}

#[test]
fn most_specific_translation_wins() {
    let map = fixture(&[
        ("Name", "Foo"),
        ("Name[de]", "DE"),
        ("Name[de_DE]", "DE-DE"),
        ("Name[de_DE@euro]", "DE-DE-EURO"),
        ("Name[de@euro]", "DE-EURO"),
    ]);
    let locale = parse_locale("de_DE.UTF-8@euro").unwrap();
    assert_eq!(
        get_localized(get(&map), "Name", Some(&locale)).as_deref(),
        Some("DE-DE-EURO")
    );
}

#[test]
fn lookup_falls_back_down_the_table_then_default() {
    let map = fixture(&[("Name", "Foo"), ("Name[de]", "DE")]);
    let locale = parse_locale("de_DE.UTF-8@euro").unwrap();
    assert_eq!(
        get_localized(get(&map), "Name", Some(&locale)).as_deref(),
        Some("DE")
    );
    let map = fixture(&[("Name", "Foo")]);
    assert_eq!(
        get_localized(get(&map), "Name", Some(&locale)).as_deref(),
        Some("Foo")
    );
    let map = fixture(&[("Name[de]", "DE")]);
    assert_eq!(
        get_localized(get(&map), "Name", Some(&locale)).as_deref(),
        Some("DE")
    );
}

#[test]
fn missing_everywhere_is_none() {
    let map = fixture(&[("Comment", "Hi")]);
    let locale = parse_locale("fr_FR.UTF-8").unwrap();
    assert_eq!(get_localized(get(&map), "Name", Some(&locale)), None);
    assert_eq!(get_localized(get(&map), "Name", None), None);
}

#[test]
fn no_locale_reads_the_default() {
    let map = fixture(&[("Name", "Foo"), ("Name[de]", "DE")]);
    assert_eq!(
        get_localized(get(&map), "Name", None).as_deref(),
        Some("Foo")
    );
}

#[test]
fn never_matches_a_country_or_modifier_the_locale_lacks() {
    // Locale `de`: `Name[de_DE]` and `Name[de@euro]` must not match.
    let map = fixture(&[
        ("Name", "Foo"),
        ("Name[de_DE]", "WRONG"),
        ("Name[de@euro]", "WRONG"),
    ]);
    let locale = parse_locale("de").unwrap();
    assert_eq!(
        get_localized(get(&map), "Name", Some(&locale)).as_deref(),
        Some("Foo")
    );
    // Locale `de_DE`: `Name[de@euro]` must not match.
    let map = fixture(&[("Name", "Foo"), ("Name[de@euro]", "WRONG")]);
    let locale = parse_locale("de_DE.UTF-8").unwrap();
    assert_eq!(
        get_localized(get(&map), "Name", Some(&locale)).as_deref(),
        Some("Foo")
    );
}

#[test]
fn broken_translation_falls_back_to_default() {
    let map = fixture(&[("Name", "Foo"), ("Name[de]", r"br\oken\q")]);
    let locale = parse_locale("de").unwrap();
    assert_eq!(
        get_localized(get(&map), "Name", Some(&locale)).as_deref(),
        Some("Foo")
    );
}

#[test]
fn escapes_apply_to_translations() {
    let map = fixture(&[("Name[de]", r"Gr\soe\\e")]);
    let locale = parse_locale("de").unwrap();
    assert_eq!(
        get_localized(get(&map), "Name", Some(&locale)).as_deref(),
        Some("Gr oe\\e")
    );
}

#[test]
fn modifier_without_country_matches_only_modifier_forms() {
    // `fr@euro`: the table is `fr@euro`, `fr`. `fr_FR` never matches.
    let locale = parse_locale("fr@euro").unwrap();
    assert_eq!(candidates(&locale), vec!["fr@euro", "fr"]);
    let map = fixture(&[
        ("Name", "Foo"),
        ("Name[fr_FR]", "WRONG"),
        ("Name[fr@euro]", "EURO"),
    ]);
    assert_eq!(
        get_localized(get(&map), "Name", Some(&locale)).as_deref(),
        Some("EURO")
    );
}
