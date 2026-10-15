//! Tests for `extends`: overrides win, chains merge, cycles and deep
//! chains refuse, installed looks are never a base.

use std::collections::BTreeMap;

use crate::resolve::resolve;
use crate::schema::{Origin, parse_look_toml};

use crate::schema::tests::minimal_toml;

fn sources(pairs: &[(&str, &str, Origin)]) -> BTreeMap<String, (crate::schema::RawLook, Origin)> {
    pairs
        .iter()
        .map(|(name, text, origin)| {
            let raw =
                parse_look_toml(text.as_bytes()).unwrap_or_else(|e| panic!("{name} parses: {e:?}"));
            (name.to_string(), (raw, *origin))
        })
        .collect()
}

#[test]
fn child_overrides_base() {
    let base = minimal_toml();
    let child = base
        .replacen(
            "name = \"moonrise\"",
            "name = \"child\"\nextends = \"moonrise\"",
            1,
        )
        .replacen("#2B3648", "#000000", 1);
    let map = sources(&[
        ("moonrise", &base, Origin::Shipped),
        ("child", &child, Origin::User),
    ]);
    let resolved = resolve(&map, "child").expect("child resolves");
    assert_eq!(resolved.name, "child");
    assert_eq!(resolved.palette.background, "#000000");
    // Untouched tokens inherit.
    assert_eq!(resolved.palette.foreground, "#F6EEDC");
    assert_eq!(resolved.cursor.theme, "Mocu");
}

#[test]
fn unknown_base_refused() {
    let orphan = minimal_toml().replacen(
        "name = \"moonrise\"",
        "name = \"orphan\"\nextends = \"missing\"",
        1,
    );
    let map = sources(&[("orphan", &orphan, Origin::User)]);
    assert!(resolve(&map, "orphan").is_err());
}

#[test]
fn self_extends_refused_at_parse() {
    let text = minimal_toml().replacen(
        "name = \"moonrise\"",
        "name = \"moonrise\"\nextends = \"moonrise\"",
        1,
    );
    assert!(parse_look_toml(text.as_bytes()).is_err());
}

#[test]
fn cycle_refused() {
    let a = minimal_toml().replacen("name = \"moonrise\"", "name = \"a\"\nextends = \"b\"", 1);
    let b = minimal_toml().replacen("name = \"moonrise\"", "name = \"b\"\nextends = \"a\"", 1);
    let map = sources(&[("a", &a, Origin::User), ("b", &b, Origin::User)]);
    let err = resolve(&map, "a").expect_err("cycle resolves");
    assert!(err.reason.contains("cycle"), "unexpected: {err}");
}

#[test]
fn deep_chain_refused() {
    let mut pairs: Vec<(String, String, Origin)> = Vec::new();
    for i in 0..11 {
        let name = format!("look{i:02}");
        let text = if i == 10 {
            minimal_toml().replacen("name = \"moonrise\"", &format!("name = \"{name}\""), 1)
        } else {
            minimal_toml().replacen(
                "name = \"moonrise\"",
                &format!("name = \"{name}\"\nextends = \"look{:02}\"", i + 1),
                1,
            )
        };
        pairs.push((name, text, Origin::User));
    }
    let map: BTreeMap<_, _> = pairs
        .iter()
        .map(|(name, text, origin)| {
            let raw = parse_look_toml(text.as_bytes()).unwrap();
            (name.clone(), (raw, *origin))
        })
        .collect();
    let err = resolve(&map, "look00").expect_err("11-deep chain resolves");
    assert!(err.reason.contains("deeper"), "unexpected: {err}");
}

#[test]
fn installed_look_never_extended() {
    let base = minimal_toml();
    let child = base.replacen(
        "name = \"moonrise\"",
        "name = \"child\"\nextends = \"moonrise\"",
        1,
    );
    let map = sources(&[
        ("moonrise", &base, Origin::Installed),
        ("child", &child, Origin::User),
    ]);
    let err = resolve(&map, "child").expect_err("installed base resolves");
    assert!(err.reason.contains("never extended"), "unexpected: {err}");
}

#[test]
fn installed_look_applies_but_never_extends() {
    // Finding 4: the `Origin::Installed` refusal ran on the root too, so an
    // installed look could not even be resolved to apply it. Design 3.1:
    // installed looks can be applied but never extended.
    let base = minimal_toml();
    let map = sources(&[("moonrise", &base, Origin::Installed)]);
    resolve(&map, "moonrise").expect("installed root refuses to apply");

    let child = base.replacen(
        "name = \"moonrise\"",
        "name = \"child\"\nextends = \"moonrise\"",
        1,
    );
    let map = sources(&[
        ("moonrise", &base, Origin::Installed),
        ("child", &child, Origin::User),
    ]);
    let err = resolve(&map, "child").expect_err("installed base resolves");
    assert!(err.reason.contains("never extended"), "unexpected: {err}");
}

#[test]
fn incomplete_chain_reports_missing_token() {
    // A base missing required tokens fails at resolution, naming the token.
    let thin = r##"
schema = 1
name = "thin"
title = "Thin"

[palette]
background = "#000000"
"##;
    let raw = parse_look_toml(thin.as_bytes()).expect("thin parses");
    let map = BTreeMap::from([("thin".to_string(), (raw, Origin::User))]);
    let err = resolve(&map, "thin").expect_err("thin resolves");
    assert!(!err.field.is_empty(), "missing token unnamed: {err}");
}
