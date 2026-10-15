//! Tests for contrast: the design's own table reproduces, shipped looks
//! refuse, user looks warn, dim and urgent never refuse.

use std::collections::BTreeMap;

use super::*;
use crate::resolve::resolve;
use crate::schema::{Origin, parse_look_toml};

fn moonrise() -> ResolvedLook {
    let text = crate::schema::tests::minimal_toml();
    let raw = parse_look_toml(text.as_bytes()).expect("minimal parses");
    let map = BTreeMap::from([("moonrise".to_string(), (raw, Origin::Shipped))]);
    resolve(&map, "moonrise").expect("minimal resolves")
}

#[test]
fn design_table_reproduces_to_01() {
    // Design section 3.5: moonrise fg/bg 10.5, dim/bg 3.79, urgent/bg 4.45.
    let look = moonrise();
    let report = check(&look, Origin::Shipped);
    let get = |name: &str| report.pairs.iter().find(|p| p.name == name).unwrap().ratio;
    assert!(
        (get("foreground/background") - 10.5).abs() < 0.15,
        "fg/bg = {}",
        get("foreground/background")
    );
    assert!(
        (get("dim/background") - 3.79).abs() < 0.15,
        "dim/bg = {}",
        get("dim/background")
    );
    assert!(
        (get("urgent/background") - 4.45).abs() < 0.15,
        "urgent/bg = {}",
        get("urgent/background")
    );
}

#[test]
fn moonrise_passes() {
    let look = moonrise();
    assert!(check(&look, Origin::Shipped).refusals().is_empty());
}

#[test]
fn low_contrast_refuses_shipped_warns_user() {
    let text = crate::schema::tests::minimal_toml().replacen("#F6EEDC", "#2B3649", 1); // foreground ~= background
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let shipped = BTreeMap::from([("x".to_string(), (raw.clone(), Origin::Shipped))]);
    let user = BTreeMap::from([("x".to_string(), (raw, Origin::User))]);
    let shipped = resolve(&shipped, "x").expect("resolves");
    let user = resolve(&user, "x").expect("resolves");
    assert!(!check(&shipped, Origin::Shipped).refusals().is_empty());
    assert!(check(&user, Origin::User).refusals().is_empty());
}

#[test]
fn dim_and_urgent_never_refuse() {
    // music-desk dim 1.57, urgent 2.88: both far under 3.0, and the look
    // must still apply.
    let text = crate::schema::tests::minimal_toml()
        .replacen("#9C8B95", "#3A4A5A", 1)
        .replacen("#E87F6A", "#445566", 1);
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::Shipped))]);
    let look = resolve(&map, "x").expect("resolves");
    let report = check(&look, Origin::Shipped);
    assert!(report.refusals().is_empty());
    assert_eq!(report.warnings().len(), 2);
}

#[test]
fn malformed_colors_never_panic() {
    // `ResolvedLook` fields are public, so a caller can hand `check` colors
    // the grammar would have refused. That must measure (as black), never
    // panic on a slice or a parse.
    for bad in ["", "#", "#12", "blue", "#GGGGGG", "＃２Ｂ３６４８"] {
        let _ = luminance(bad);
        let _ = ratio(bad, "#FFFFFF");
        let _ = ratio("#FFFFFF", bad);
    }
    let mut look = moonrise();
    look.palette.foreground = "blue".to_string();
    let _ = check(&look, Origin::Shipped);
}
