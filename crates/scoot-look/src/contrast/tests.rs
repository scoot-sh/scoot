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
    // dim/background, dim/surface (the surface defaults to the background),
    // urgent/background and border/background (the minimal look's border is
    // its dim).
    assert_eq!(report.warnings().len(), 4);
}

fn pair(report: &ContrastReport, name: &str) -> ContrastPair {
    report
        .pairs
        .iter()
        .find(|p| p.name == name)
        .cloned()
        .unwrap_or_else(|| panic!("no {name} pair"))
}

#[test]
fn dim_on_surface_is_measured_against_the_surface_and_reports() {
    // The surface differs from the background: the pair uses the surface.
    // 4.5 is the text bar; below it warns (never refuses), on every origin.
    let text = crate::schema::tests::minimal_toml().replacen(
        "[palette]",
        "[palette]\nsurface = \"#FFFFFF\"",
        1,
    );
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::Shipped))]);
    let look = resolve(&map, "x").expect("resolves");
    let report = check(&look, Origin::Shipped);
    let dim = pair(&report, "dim/surface");
    assert_eq!(dim.background, "#FFFFFF");
    assert_eq!(dim.threshold, 4.5);
    assert!(!dim.refuses);
    assert_eq!(dim.ratio, ratio("#9C8B95", "#FFFFFF"));
    assert!(report.warnings().iter().any(|p| p.name == "dim/surface"));
    assert!(report.refusals().is_empty());
}

#[test]
fn a_gradient_fill_measures_dim_at_its_worse_stop() {
    // moonrise's dim is 3.79 on its flat background; the gradient lightens
    // the top by 26/255 toward white, which is the worse stop for light text.
    let text = crate::schema::tests::minimal_toml();
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::Shipped))]);
    let mut look = resolve(&map, "x").expect("resolves");
    let flat = pair(&check(&look, Origin::Shipped), "dim/surface");
    assert_eq!(flat.background, look.palette.surface);
    look.effects.fill = Some("gradient".to_string());
    let gradient = pair(&check(&look, Origin::Shipped), "dim/surface");
    assert_eq!(gradient.background, "#414a5b");
    assert!(gradient.ratio < flat.ratio, "{gradient:?} vs {flat:?}");
    // A solid fill is the flat surface.
    look.effects.fill = Some("solid".to_string());
    assert_eq!(
        pair(&check(&look, Origin::Shipped), "dim/surface").background,
        flat.background
    );
}

#[test]
fn a_light_gradient_darkens_the_bottom_stop() {
    assert_eq!(gradient_stops("#FCFBFB")[1], "#e8e7e7");
    assert_eq!(gradient_stops("#2B3648")[1], "#414a5b");
    // A malformed surface reads as black and never panics.
    assert_eq!(gradient_stops("nope")[0], "nope");
}

#[test]
fn the_inactive_ring_is_held_to_three_to_one_and_only_reports() {
    let text = crate::schema::tests::minimal_toml().replacen(
        "[palette]",
        "[palette]\nborder = \"#2C3749\"",
        1,
    );
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::Shipped))]);
    let look = resolve(&map, "x").expect("resolves");
    let report = check(&look, Origin::Shipped);
    let ring = pair(&report, "border/background");
    assert_eq!(ring.threshold, 3.0);
    assert!(ring.ratio < 3.0 && !ring.refuses);
    assert!(report.refusals().is_empty());
}

#[test]
fn the_inactive_ring_names_the_border_color_as_its_foreground() {
    // The surviving mutation from the dark-ring review: swapping which
    // color the `border/background` pair reports as its foreground passed
    // every test. The warning names the ring color, so pin it: the pair's
    // foreground is the look's `border`, its background the `background`.
    // (The minimal fixture defaults `border` to `dim`, which would not
    // catch the swap, so this sets a distinctive ring color first.)
    let text = crate::schema::tests::minimal_toml().replacen(
        "[palette]",
        "[palette]\nborder = \"#2C3749\"",
        1,
    );
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::Shipped))]);
    let look = resolve(&map, "x").expect("resolves");
    assert_ne!(look.palette.border, look.palette.dim);
    let report = check(&look, Origin::Shipped);
    let ring = pair(&report, "border/background");
    assert_eq!(ring.foreground, "#2C3749");
    assert_eq!(ring.foreground, look.palette.border);
    assert_eq!(ring.background, look.palette.background);
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

#[test]
fn format_ratio_truncates_toward_zero_to_two_decimals() {
    // Truncation (not rounding) so a failing pair can never print as its
    // bar: 4.496 against a 4.5 bar prints "4.49", never "4.50". The `1e-9`
    // absorbs binary float error so exact cent values still print exact.
    for (input, want) in [
        (4.4999, "4.49"),
        (4.496, "4.49"),
        (2.995, "2.99"),
        (3.0, "3.00"),
        (21.0, "21.00"),
        (1.0, "1.00"),
        (4.5, "4.50"),
        (3.12, "3.12"),
    ] {
        assert_eq!(format_ratio(input), want, "format_ratio({input})");
    }
}
