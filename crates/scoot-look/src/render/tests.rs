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
        bar_drop_sections: Vec::new(),
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
        "[apps.scootbar]\n\"colors.accent\" = \"#000000\"\n\"bar.radius\" = 3\n\"bar.padding\" = 7\n\"workspaces.inactive-color\" = \"#111111\"\n[cursor]",
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
    assert_eq!(parsed["bar"]["padding"].as_integer(), Some(7));
    assert_eq!(
        parsed["workspaces"]["inactive-color"].as_str(),
        Some("#111111")
    );
}

#[test]
fn dropped_module_sections_never_render() {
    // Tokens for modules the installed bar lacks are dropped at render:
    // the `[workspaces]` table vanishes while `[colors]` and `[bar]`
    // survive, and the fragment still reparses.
    let text = crate::schema::tests::minimal_toml().replacen(
        "[cursor]",
        "[apps.scootbar]\n\"workspaces.inactive-color\" = \"#111111\"\n[cursor]",
        1,
    );
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::User))]);
    let look = resolve(&map, "x").expect("resolves");
    let mut without = inputs();
    without.bar_drop_sections = vec!["workspaces".to_owned()];
    let frag = render_bar(&look, &without).expect("renders");
    assert!(
        !frag.contains("[workspaces]"),
        "dropped section renders:\n{frag}"
    );
    assert!(
        !frag.contains("inactive-color"),
        "dropped token renders:\n{frag}"
    );
    assert!(frag.contains("[colors]"), "base table lost:\n{frag}");
    let _: toml::Table = toml::from_str(&frag).expect("bar fragment reparses");
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
            "wallpaper.output.My Monitor.color".to_string(),
            toml::Value::String("#000000".to_string()),
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

#[test]
fn bar_radius_clamped_to_half_default_height() {
    // Round 2 finding 2: scootbar refuses `bar.radius` past half the
    // height (`crates/scootbar/src/config.rs:1069-1081`; default height
    // 28 from `crates/scootbar/src/bar.rs:28-30`, so the limit is 14),
    // while `shape.radius` runs 0 to 64. The derived radius clamps.
    let text = crate::schema::tests::minimal_toml().replacen("radius = 14", "radius = 64", 1);
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::User))]);
    let look = resolve(&map, "x").expect("resolves");
    let frag = render_bar(&look, &inputs()).expect("renders");
    let parsed: toml::Table = toml::from_str(&frag).expect("reparses");
    assert_eq!(parsed["bar"]["radius"].as_integer(), Some(14));
    // The boundary passes through untouched.
    let frag = render_bar(&moonrise(), &inputs()).expect("renders");
    let parsed: toml::Table = toml::from_str(&frag).expect("reparses");
    assert_eq!(parsed["bar"]["radius"].as_integer(), Some(14));
}

#[test]
fn bar_radius_override_clamped_at_render() {
    // Round 2 finding 2, second half: a `bar.radius` override parses
    // (scootbar accepts it under a taller user bar) but the fragment
    // must still pass the parser under the default height, so render
    // clamps it too.
    let text = crate::schema::tests::minimal_toml().replacen(
        "[cursor]",
        "[apps.scootbar]\n\"bar.radius\" = 512\n[cursor]",
        1,
    );
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::User))]);
    let look = resolve(&map, "x").expect("resolves");
    let frag = render_bar(&look, &inputs()).expect("renders");
    let parsed: toml::Table = toml::from_str(&frag).expect("reparses");
    assert_eq!(parsed["bar"]["radius"].as_integer(), Some(14));
}

#[test]
fn bar_font_omitted_without_a_resolved_path() {
    // Round 2 finding 3: scootbar's `font` is a file path
    // (`crates/scootbar/src/config.rs:87-90`, `font.rs:1`), not the
    // family name `fonts.ui` holds. Without `--ui-font-file` the
    // fragment omits both keys and the bar falls back to its
    // well-known paths; a family name would fail the font load.
    let frag = render_bar(&moonrise(), &inputs()).expect("renders");
    for line in frag.lines() {
        let key = line.split('=').next().unwrap_or("").trim();
        assert_ne!(key, "font", "family name emitted:\n{frag}");
        assert_ne!(key, "fallback-fonts", "fallbacks emitted:\n{frag}");
    }
    let with_font = RenderInputs {
        ui_font_file: Some("/fonts/DejaVuSans.ttf".to_string()),
        ..inputs()
    };
    let frag = render_bar(&moonrise(), &with_font).expect("renders");
    assert!(
        frag.contains("font = \"/fonts/DejaVuSans.ttf\""),
        "resolved font missing:\n{frag}"
    );
}

#[test]
fn render_checks_override_values() {
    // Round 2 finding 5: the parse gate refuses bad override values,
    // but a library caller can hand-build a `ResolvedLook` past it, so
    // the render path checks values too. `i64::MAX` overflows scoot's
    // `i32` `corner_radius`, which would discard the whole file there.
    let mut look = moonrise();
    look.apps = BTreeMap::from([(
        "scoot".to_string(),
        BTreeMap::from([(
            "appearance.corner_radius".to_string(),
            toml::Value::Integer(i64::MAX),
        )]),
    )]);
    assert!(render_scoot(&look, &inputs()).is_err());
}

/// Moonrise's 16 ANSI hues, the base16 terminal order foot reads as
/// `regular0-7` then `bright0-7` (`looks/moonrise/look.toml`).
const TERMINAL_COLORS: [&str; 16] = [
    "#1a2230", "#c96a63", "#8fa382", "#e8a34c", "#7c8fb0", "#9a7b94", "#7e9bb0", "#c9bfae",
    "#5e6b7f", "#e87f6a", "#a9bd9b", "#ffd54a", "#93a8c8", "#b595ad", "#a8c4d8", "#f6eedc",
];

fn terminal_toml() -> String {
    let colors = TERMINAL_COLORS
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    crate::schema::tests::minimal_toml().replacen(
        "[fonts]",
        &format!("[terminal]\ncolors = [{colors}]\n[fonts]"),
        1,
    )
}

fn terminal_look() -> ResolvedLook {
    let raw = parse_look_toml(terminal_toml().as_bytes()).expect("terminal look parses");
    let map = BTreeMap::from([("moonrise".to_string(), (raw, Origin::Shipped))]);
    resolve(&map, "moonrise").expect("terminal look resolves")
}

#[test]
fn foot_dark_section_holds_the_terminal_hues_in_ansi_order() {
    let frag = render_foot(&terminal_look())
        .expect("renders")
        .expect("a terminal table renders a fragment");
    assert!(frag.contains("[colors-dark]"), "wrong section:\n{frag}");
    assert!(!frag.contains("[colors-light]"), "both sections:\n{frag}");
    assert!(!frag.contains("[colors]\n"), "bare colors section:\n{frag}");
    // ANSI 0-7 then 8-15, foot's hex without `#`.
    for (i, color) in TERMINAL_COLORS.iter().enumerate() {
        let (key, _) = if i < 8 {
            (format!("regular{i}"), i)
        } else {
            (format!("bright{}", i - 8), i)
        };
        let line = format!("{key}={}", color.trim_start_matches('#'));
        assert!(frag.lines().any(|l| l == line), "missing {line}:\n{frag}");
    }
    assert!(
        frag.lines()
            .filter(|l| !l.starts_with('#'))
            .all(|l| !l.contains('#')),
        "a `#` escapes into foot.ini:\n{frag}"
    );
}

#[test]
fn foot_light_look_uses_colors_light() {
    let text = terminal_toml().replacen("polarity = \"dark\"", "polarity = \"light\"", 1);
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("moonrise".to_string(), (raw, Origin::Shipped))]);
    let look = resolve(&map, "moonrise").expect("resolves");
    let frag = render_foot(&look)
        .expect("renders")
        .expect("a terminal table renders a fragment");
    assert!(frag.contains("[colors-light]"), "wrong section:\n{frag}");
    assert!(!frag.contains("[colors-dark]"), "both sections:\n{frag}");
}

#[test]
fn foot_alpha_comes_from_opacity_terminal() {
    // The shipped example writes `alpha=0.80` beside the colors; the
    // default opacity renders full.
    let text = terminal_toml().replacen("[shape]", "[opacity]\nterminal = 0.8\n[shape]", 1);
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("moonrise".to_string(), (raw, Origin::Shipped))]);
    let look = resolve(&map, "moonrise").expect("resolves");
    let frag = render_foot(&look)
        .expect("renders")
        .expect("a terminal table renders a fragment");
    assert!(
        frag.lines().any(|l| l == "alpha=0.80"),
        "alpha missing:\n{frag}"
    );
    let frag = render_foot(&terminal_look())
        .expect("renders")
        .expect("a terminal table renders a fragment");
    assert!(
        frag.lines().any(|l| l == "alpha=1.00"),
        "default alpha missing:\n{frag}"
    );
}

#[test]
fn foot_absent_without_a_terminal_table() {
    // No `[terminal]` table means no foot fragment, never a palette
    // synthesized into one (design section 3.3).
    assert!(render_foot(&moonrise()).expect("renders").is_none());
}

#[test]
fn foot_refuses_a_hand_built_bad_palette() {
    // The parse gate refuses these, but a library caller can hand-build
    // a `ResolvedLook` past it, so the renderer checks too: a newline
    // in a hue must never open a new foot section.
    let mut look = terminal_look();
    look.terminal = Some(vec![
        "x\n[main]\nfont = pwned".to_string(),
        "x".to_string(),
        "x".to_string(),
        "x".to_string(),
        "x".to_string(),
        "x".to_string(),
        "x".to_string(),
        "x".to_string(),
        "x".to_string(),
        "x".to_string(),
        "x".to_string(),
        "x".to_string(),
        "x".to_string(),
        "x".to_string(),
        "x".to_string(),
        "x".to_string(),
    ]);
    let err = render_foot(&look).expect_err("injected hue renders");
    assert_eq!(err.field, "terminal.colors[0]");
    look.terminal = Some(vec!["#000000".to_string(); 15]);
    let err = render_foot(&look).expect_err("fifteen hues render");
    assert_eq!(err.field, "terminal.colors");
    look.opacity.terminal = f64::NAN;
    look.terminal = Some(TERMINAL_COLORS.iter().map(|s| s.to_string()).collect());
    let err = render_foot(&look).expect_err("NaN alpha renders");
    assert_eq!(err.field, "opacity.terminal");
}
