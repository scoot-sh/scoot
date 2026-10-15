//! Tests for the renderers: the moonrise fragments read as designed,
//! `popup-radius` is gated on the feature, `cursor.accent = "none"` omits
//! the key, overrides merge, and non-allowlisted overrides refuse.

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

fn inputs() -> RenderInputs {
    RenderInputs {
        wallpaper_abs: Some("/cache/wallpaper.png".to_string()),
        ui_font_file: None,
        bar_popup: true,
    }
}

#[test]
fn scoot_fragment() {
    let frag = render_scoot(&moonrise(), &inputs()).expect("renders");
    for line in [
        "focus_ring_width = 2",
        "focus_ring_inactive_width = 2",
        "focus_ring_active_color = \"#FFA45C\"",
        "focus_ring_inactive_color = \"#9C8B95\"",
        "background_color = \"#2B3648\"",
        "corner_radius = 14",
        "cursor_theme = \"Mocu\"",
        "cursor_size = 24",
        "cursor_color = \"#FF9A49\"",
        "image = \"/cache/wallpaper.png\"",
    ] {
        assert!(frag.contains(line), "missing {line}:\n{frag}");
    }
    assert!(!frag.contains("autostart"), "fragment leaks autostart");
    assert!(!frag.contains("binds"), "fragment leaks binds");
    // The parsed fragment is valid TOML.
    let _: toml::Value = toml::from_str(&frag).expect("fragment is TOML");
}

#[test]
fn bar_fragment() {
    let frag = render_bar(&moonrise(), &inputs()).expect("renders");
    for line in [
        "background = \"#2B3648\"",
        "accent = \"#FFA45C\"",
        "radius = 14",
        "popup-radius = 14",
        "font-size = 13",
    ] {
        assert!(frag.contains(line), "missing {line}:\n{frag}");
    }
    let _: toml::Value = toml::from_str(&frag).expect("fragment is TOML");
}

#[test]
fn popup_radius_gated_on_feature() {
    let no_popup = RenderInputs {
        bar_popup: false,
        ..inputs()
    };
    let frag = render_bar(&moonrise(), &no_popup).expect("renders");
    assert!(
        !frag.contains("popup-radius"),
        "popup-radius without the feature:\n{frag}"
    );
}

#[test]
fn cursor_none_omits_color() {
    let text = crate::schema::tests::minimal_toml().replacen(
        "accent = \"#FF9A49\"",
        "accent = \"none\"",
        1,
    );
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::User))]);
    let look = resolve(&map, "x").expect("resolves");
    let frag = render_scoot(&look, &inputs()).expect("renders");
    assert!(
        !frag.contains("cursor_color"),
        "none accent emits a color:\n{frag}"
    );
}

#[test]
fn image_wallpaper_needs_abs_path() {
    let no_abs = RenderInputs {
        wallpaper_abs: None,
        ..inputs()
    };
    let err = render_scoot(&moonrise(), &no_abs).expect_err("renders without a path");
    assert_eq!(err.field, "wallpaper.image");
}

#[test]
fn overrides_merge_and_refuse() {
    let text = crate::schema::tests::minimal_toml().replacen(
        "[cursor]",
        "[apps.scootbar]\n\"workspaces.inactive-color\" = \"#B595AD\"\n[cursor]",
        1,
    );
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::User))]);
    let look = resolve(&map, "x").expect("resolves");
    let frag = render_bar(&look, &inputs()).expect("renders");
    assert!(
        frag.contains("inactive-color = \"#B595AD\""),
        "override missing:\n{frag}"
    );

    let bad = crate::schema::tests::minimal_toml().replacen(
        "[cursor]",
        "[apps.scoot]\n\"binds.foo\" = \"x\"\n[cursor]",
        1,
    );
    // Non-allowlisted keys never reach the renderer: the parse refuses
    // them (finding 3), so the renderer's own refusal is only a backstop.
    // A hand-built look still hits it.
    assert!(parse_look_toml(bad.as_bytes()).is_err());
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::User))]);
    let mut look = resolve(&map, "x").expect("resolves");
    look.apps = BTreeMap::from([(
        "scoot".to_string(),
        BTreeMap::from([("binds.foo".to_string(), toml::Value::String("x".into()))]),
    )]);
    assert!(render_scoot(&look, &inputs()).is_err());
}

#[test]
fn overrides_merge_into_base_tables() {
    // Finding 5: overrides used to append duplicate `[appearance]` /
    // `[wallpaper]` tables (which do not reparse) and `bar.*` overrides
    // landed at `bar.bar.*` (which the bar's deny-unknown-fields refuses).
    // Every rendered fragment with overrides must reparse, with each
    // section emitted once and the override winning.
    let text = crate::schema::tests::minimal_toml()
        .replacen(
            "[cursor]",
            "[apps.scoot]\n\"appearance.background_color\" = \"#000000\"\n\"wallpaper.mode\" = \"tile\"\n[cursor]",
            1,
        );
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::User))]);
    let look = resolve(&map, "x").expect("resolves");
    let frag = render_scoot(&look, &inputs()).expect("renders");
    assert_eq!(
        frag.matches("[appearance]").count(),
        1,
        "duplicate [appearance]:\n{frag}"
    );
    assert_eq!(
        frag.matches("[wallpaper]").count(),
        1,
        "duplicate [wallpaper]:\n{frag}"
    );
    let parsed: toml::Table = toml::from_str(&frag).expect("scoot fragment reparses");
    assert_eq!(
        parsed["appearance"]["background_color"].as_str(),
        Some("#000000")
    );
    assert_eq!(parsed["wallpaper"]["mode"].as_str(), Some("tile"));

    let text = crate::schema::tests::minimal_toml().replacen(
        "[cursor]",
        "[apps.scootbar]\n\"colors.accent\" = \"#000000\"\n\"bar.radius\" = 3\n\"bar.fallback-fonts\" = [\"DejaVu Sans\"]\n\"workspaces.inactive-color\" = \"#111111\"\n[cursor]",
        1,
    );
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::User))]);
    let look = resolve(&map, "x").expect("resolves");
    let frag = render_bar(&look, &inputs()).expect("renders");
    assert_eq!(
        frag.matches("[colors]").count(),
        1,
        "duplicate [colors]:\n{frag}"
    );
    assert_eq!(frag.matches("[bar]").count(), 1, "duplicate [bar]:\n{frag}");
    assert!(!frag.contains("bar.bar."), "misplaced bar key:\n{frag}");
    let parsed: toml::Table = toml::from_str(&frag).expect("bar fragment reparses");
    assert_eq!(parsed["colors"]["accent"].as_str(), Some("#000000"));
    assert_eq!(parsed["bar"]["radius"].as_integer(), Some(3));
    assert_eq!(
        parsed["bar"]["fallback-fonts"].as_array().map(|a| a.len()),
        Some(1)
    );
    assert_eq!(
        parsed["workspaces"]["inactive-color"].as_str(),
        Some("#111111")
    );
}

#[test]
fn pt_to_px_follows_the_bar_rule() {
    // pixels = clamp(1, 256, (pt * 4 + 1) / 3), floored.
    assert_eq!(pt_to_px(10.0), 13);
    assert_eq!(pt_to_px(10.5), 14);
    assert_eq!(pt_to_px(12.0), 16);
    assert_eq!(pt_to_px(1000.0), 256);
    assert_eq!(pt_to_px(0.5), 1);
}

#[test]
fn hostile_keys_never_render_raw() {
    // Finding 1, both halves: hostile keys are refused by the allowlist
    // before the renderer sees them, and the key quoter the renderer uses
    // quotes every one of the design's probe characters (newline, `=`,
    // `[`, quote), so even a grammar bypass cannot open a new table.
    for hostile in [
        "wallpaper.output.x = 1\n[autostart]\ncommand",
        "wallpaper.output.x\n[autostart]",
        "colors.accent\n[autostart]",
        "a=b",
        "a[",
        "a\"b",
        "a\\b",
        "a b",
        "a\nb",
    ] {
        let quoted = toml_key(hostile);
        assert!(
            quoted.starts_with('"'),
            "{hostile:?} renders bare: {quoted}"
        );
        // The quoted key reparses to exactly itself.
        let parsed: toml::Table =
            toml::from_str(&format!("{quoted} = 1")).expect("quoted key is TOML");
        assert_eq!(parsed.get(hostile), Some(&toml::Value::Integer(1)));
    }
    assert_eq!(toml_key("radius"), "radius");
    assert_eq!(
        toml_dotted("output.My Monitor.image"),
        "output.\"My Monitor\".image"
    );

    // End to end: a valid override whose output name needs quoting renders
    // quoted and reparses, with no duplicate table.
    let mut look = moonrise();
    look.apps = BTreeMap::from([(
        "scoot".to_string(),
        BTreeMap::from([(
            "wallpaper.output.My Monitor.image".to_string(),
            toml::Value::String("wallpaper.png".to_string()),
        )]),
    )]);
    let frag = render_scoot(&look, &inputs()).expect("renders");
    let parsed: toml::Value = toml::from_str(&frag).expect("fragment is TOML");
    assert!(
        frag.contains("\"My Monitor\""),
        "output name not quoted:\n{frag}"
    );
    assert!(!frag.contains("[autostart]"), "injection:\n{frag}");
    let _ = parsed;

    // A scootbar key through the scoot renderer is refused, not misplaced.
    let mut look = moonrise();
    look.apps = BTreeMap::from([(
        "scoot".to_string(),
        BTreeMap::from([("bar.radius".to_string(), toml::Value::Integer(3))]),
    )]);
    assert!(render_scoot(&look, &inputs()).is_err());
}
