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
        omit_cursor: false,
        foot_legacy: false,
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
fn color_wallpaper_carries_no_image_keys() {
    // `mode` and `fill` apply to an image only: scootbg refuses them
    // beside a color (`ImageOnly`), so a color wallpaper renders `color`
    // and `transition` alone, never `image`, `mode` or `fill`.
    let text = crate::schema::tests::minimal_toml().replacen(
        "[wallpaper]\nimage = \"wallpaper.png\"\nmode = \"fill\"",
        "[wallpaper]\ncolor = \"#101014\"\ntransition = \"fade\"",
        1,
    );
    let raw = parse_look_toml(text.as_bytes()).expect("color look parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::User))]);
    let look = resolve(&map, "x").expect("color look resolves");
    let frag = render_scoot(&look, &inputs()).expect("renders");
    let parsed: toml::Value = toml::from_str(&frag).expect("fragment is TOML");
    let wallpaper = &parsed["wallpaper"];
    assert_eq!(
        wallpaper["color"].as_str(),
        Some("#101014"),
        "color renders:\n{frag}"
    );
    assert_eq!(
        wallpaper["transition"].as_str(),
        Some("fade"),
        "transition renders:\n{frag}"
    );
    for key in ["image", "mode", "fill"] {
        assert!(
            wallpaper.get(key).is_none(),
            "color wallpaper renders `{key}`:\n{frag}"
        );
    }
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
fn omit_cursor_leaves_no_cursor_keys() {
    let omitted = RenderInputs {
        omit_cursor: true,
        ..inputs()
    };
    let frag = render_scoot(&moonrise(), &omitted).expect("renders");
    for key in ["cursor_theme", "cursor_size", "cursor_color"] {
        assert!(!frag.contains(key), "opted-out cursor emits {key}:\n{frag}");
    }
    // Everything else still renders.
    for line in [
        "background_color = \"#2B3648\"",
        "corner_radius = 14",
        "image = \"/cache/wallpaper.png\"",
    ] {
        assert!(frag.contains(line), "missing {line}:\n{frag}");
    }
    let _: toml::Value = toml::from_str(&frag).expect("fragment is TOML");
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

#[test]
fn gtk3_css_carries_the_selection_rules() {
    // No `:root` block (GTK 3's parser dies on it, measured:
    // `Theme parsing error: gtk.css:18:1`) and no `@define-color`
    // (proven without effect on its embedded Adwaita): just the
    // selection rules, proven amber live on gtk3-widget-factory.
    let css = render_gtk3_css(&moonrise());
    for line in [
        "entry selection,",
        ".view selection,",
        "textview selection,",
        "row:selected {",
        "background-color: #FFA45C;",
        "color: #2B3648;",
    ] {
        assert!(css.contains(line), "missing {line}:\n{css}");
    }
    assert!(!css.contains(":root"), "a :root block breaks GTK 3:\n{css}");
    assert!(
        !css.contains("@define-color"),
        "defines have no effect on GTK 3:\n{css}"
    );
}

#[test]
fn gtk4_css_carries_names_and_root_vars() {
    // The same names the profile's `gtkCss` writes
    // (`nix/modules/theme-home.nix`): `@define-color` plus the `:root`
    // vars libadwaita widgets paint with. Moonrise is dark, so the
    // surfaces are its slate navy and the accent its amber.
    let css = render_gtk4_css(&moonrise());
    for line in [
        "@define-color accent_bg_color #FFA45C;",
        "@define-color accent_fg_color #2B3648;",
        "@define-color window_bg_color #2B3648;",
        "@define-color window_fg_color #F6EEDC;",
        "@define-color headerbar_border_color #9C8B95;",
        "  --accent-bg-color: #FFA45C;",
        "  --window-bg-color: #2B3648;",
        "  --headerbar-border-color: #9C8B95;",
        "  --dialog-fg-color: #F6EEDC;",
    ] {
        assert!(css.contains(line), "missing {line}:\n{css}");
    }
    assert!(css.contains(":root {"), "no :root block:\n{css}");
    // The literal hexes, never `@`-references (which resolve per provider).
    let without_names = css.replace("@define-color", "");
    assert!(
        !without_names.contains('@'),
        "an @-reference leaked:\n{css}"
    );
}

#[test]
fn qt_scheme_is_21_roles_in_qpalette_order() {
    // The profile's `qtScheme` shape: three rows of 21 `#AARRGGBB`
    // entries. Spot-check the roles the mapping promises: Window (10)
    // is the background, Highlight (12) the accent with the background
    // on top (13), inactive mirrors active, disabled dims the text.
    let scheme = render_qt_scheme(&moonrise());
    let active = scheme
        .lines()
        .find(|line| line.starts_with("active_colors="))
        .expect("active row");
    let roles: Vec<&str> = active["active_colors=".len()..].split(", ").collect();
    assert_eq!(roles.len(), 21, "not 21 roles:\n{scheme}");
    assert_eq!(roles[9], "#ff2b3648", "Base is not the background");
    assert_eq!(roles[10], "#ff2b3648", "Window is not the background");
    assert_eq!(roles[12], "#ffffa45c", "Highlight is not the accent");
    assert_eq!(
        roles[13], "#ff2b3648",
        "HighlightedText is not background-on-accent"
    );
    assert_eq!(roles[0], "#fff6eedc", "WindowText is not the foreground");
    let inactive = scheme
        .lines()
        .find(|line| line.starts_with("inactive_colors="))
        .expect("inactive row");
    assert_eq!(
        inactive["inactive_colors=".len()..],
        active["active_colors=".len()..],
        "inactive does not mirror active"
    );
    let disabled = scheme
        .lines()
        .find(|line| line.starts_with("disabled_colors="))
        .expect("disabled row");
    let dimmed: Vec<&str> = disabled["disabled_colors=".len()..].split(", ").collect();
    assert_eq!(dimmed[0], "#ff9c8b95", "disabled text is not dim");
}

#[test]
fn qt6ct_conf_names_style_and_scheme() {
    let conf = render_qt6ct_conf(&moonrise(), "/state/look/qt/scoot-look.conf").expect("renders");
    for line in [
        "style=Adwaita-Dark",
        "icon_theme=Adwaita",
        "custom_palette=true",
        "color_scheme_path=/state/look/qt/scoot-look.conf",
    ] {
        assert!(conf.contains(line), "missing {line}:\n{conf}");
    }
    // A light look takes the light style variant.
    let mut light = moonrise();
    light.polarity = crate::schema::Polarity::Light;
    let conf = render_qt6ct_conf(&light, "/state/look/qt/scoot-look.conf").expect("renders");
    assert!(
        conf.contains("style=Adwaita\n"),
        "light style wrong:\n{conf}"
    );
    // The path lands in an INI value: no quotes, no newlines, no controls.
    assert!(render_qt6ct_conf(&moonrise(), "/s\ncheme").is_err());
    assert!(render_qt6ct_conf(&moonrise(), "").is_err());
}

#[test]
fn gtk_ini_names_theme_font_cursor() {
    let ini = render_gtk_ini(&moonrise());
    for line in [
        "[Settings]",
        "gtk-theme-name=Adwaita-dark",
        "gtk-icon-theme-name=Adwaita",
        "gtk-font-name=DejaVu Sans 12",
        "gtk-cursor-theme-name=Mocu",
        "gtk-cursor-theme-size=24",
        "gtk-application-prefer-dark-theme=true",
    ] {
        assert!(ini.contains(line), "missing {line}:\n{ini}");
    }
}

#[test]
fn fontconfig_prefers_the_look_faces() {
    let conf = render_fontconfig(&moonrise());
    assert!(
        conf.contains("<string>DejaVu Sans</string>"),
        "sans missing:\n{conf}"
    );
    assert!(
        conf.contains("<string>FiraCode Nerd Font</string>"),
        "mono missing:\n{conf}"
    );
    assert!(
        conf.contains("mode=\"prepend\""),
        "not a prepend rule:\n{conf}"
    );
}

#[test]
fn fontconfig_rules_apply_only_to_a_first_choice_generic() {
    let conf = render_fontconfig(&moonrise());
    for generic in ["sans-serif", "system-ui", "monospace"] {
        let rule =
            format!("<test name=\"family\" qual=\"first\"><string>{generic}</string></test>");
        assert!(conf.contains(&rule), "{generic} not first-only:\n{conf}");
    }
    // No test may match a generic anywhere in the list: that would turn an
    // explicit named family into the look's face.
    assert!(
        !conf.contains("<test name=\"family\"><string>"),
        "an unqualified family test:\n{conf}"
    );
    assert_eq!(conf.matches("<match ").count(), 3, "{conf}");
}

#[test]
fn theme_and_style_names_follow_polarity() {
    assert_eq!(gtk_theme_name(true), "Adwaita-dark");
    assert_eq!(gtk_theme_name(false), "Adwaita");
    assert_eq!(qt_style_name(true), "Adwaita-Dark");
    assert_eq!(qt_style_name(false), "Adwaita");
    assert_eq!(font_desc("DejaVu Sans", 12.0), "DejaVu Sans 12");
    assert_eq!(font_desc("DejaVu Sans", 10.5), "DejaVu Sans 10.5");
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
    assert!(frag.contains("[main]"), "no [main]:\n{frag}");
    assert!(
        frag.contains("initial-color-theme=dark"),
        "wrong theme:\n{frag}"
    );
    // Both palettes carry the same hues: whatever theme is in effect when
    // a new terminal starts, the look's hues show.
    for section in ["[colors-dark]", "[colors-light]"] {
        assert!(frag.contains(section), "missing {section}:\n{frag}");
    }
    assert!(!frag.contains("[colors]\n"), "bare colors section:\n{frag}");
    // ANSI 0-7 then 8-15, foot's hex without `#`, twice (once per theme).
    for (i, color) in TERMINAL_COLORS.iter().enumerate() {
        let (key, _) = if i < 8 {
            (format!("regular{i}"), i)
        } else {
            (format!("bright{}", i - 8), i)
        };
        let line = format!("{key}={}", color.trim_start_matches('#'));
        assert_eq!(
            frag.lines().filter(|l| *l == line).count(),
            2,
            "missing a copy of {line}:\n{frag}"
        );
    }
    assert!(
        frag.lines()
            .filter(|l| !l.starts_with('#'))
            .all(|l| !l.contains('#')),
        "a `#` escapes into foot.ini:\n{frag}"
    );
}

#[test]
fn foot_initial_theme_follows_polarity() {
    // The effective theme selection, both polarities: a dark look starts
    // new terminals on `colors-dark`, a light look on `colors-light`.
    // (Either theme still carries the look's hues, so a user's own
    // `initial-color-theme` after the include cannot un-theme a terminal.)
    let dark = render_foot(&terminal_look())
        .expect("renders")
        .expect("a terminal table renders a fragment");
    assert_eq!(
        dark.lines()
            .filter(|l| *l == "initial-color-theme=dark")
            .count(),
        1,
        "dark look selects no single theme:\n{dark}"
    );
    assert!(
        !dark.contains("initial-color-theme=light"),
        "dark look selects light:\n{dark}"
    );
    let text = terminal_toml().replacen("polarity = \"dark\"", "polarity = \"light\"", 1);
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("moonrise".to_string(), (raw, Origin::Shipped))]);
    let look = resolve(&map, "moonrise").expect("resolves");
    let light = render_foot(&look)
        .expect("renders")
        .expect("a terminal table renders a fragment");
    assert_eq!(
        light
            .lines()
            .filter(|l| *l == "initial-color-theme=light")
            .count(),
        1,
        "light look selects no single theme:\n{light}"
    );
    assert!(
        !light.contains("initial-color-theme=dark"),
        "light look selects dark:\n{light}"
    );
    for frag in [&dark, &light] {
        for section in ["[colors-dark]", "[colors-light]"] {
            assert!(frag.contains(section), "missing {section}:\n{frag}");
        }
    }
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
    assert!(frag.contains("[colors-dark]"), "missing mirror:\n{frag}");
    assert!(
        frag.contains("initial-color-theme=light"),
        "wrong theme:\n{frag}"
    );
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
    assert_eq!(
        frag.lines().filter(|l| *l == "alpha=0.80").count(),
        2,
        "alpha not in both themes:\n{frag}"
    );
    let frag = render_foot(&terminal_look())
        .expect("renders")
        .expect("a terminal table renders a fragment");
    assert!(
        frag.lines().any(|l| l == "alpha=1.00"),
        "default alpha missing:\n{frag}"
    );
    assert_eq!(
        frag.lines().filter(|l| *l == "alpha=1.00").count(),
        2,
        "default alpha not in both themes:\n{frag}"
    );
}

#[test]
fn foot_absent_without_a_terminal_table() {
    // No `[terminal]` table means no foot fragment, never a palette
    // synthesized into one (design section 3.3).
    assert!(render_foot(&moonrise()).expect("renders").is_none());
    assert!(render_foot_legacy(&moonrise()).expect("renders").is_none());
}

#[test]
fn foot_legacy_renders_one_colors_section_for_an_old_foot() {
    // look-apply-older-foot-refuses: foot before 1.26 has no
    // `[colors-dark]`/`[colors-light]` sections and (before 1.23) no
    // `initial-color-theme` key, so it refuses the themed fragment.
    // The legacy shape carries the same hues in the one `[colors]`
    // section that era parses, with no themed keys at all.
    let frag = render_foot_legacy(&terminal_look())
        .expect("renders")
        .expect("a terminal table renders a fragment");
    assert!(frag.contains("[colors]"), "no [colors]:\n{frag}");
    assert!(
        !frag.contains("initial-color-theme"),
        "legacy keeps the refused key:\n{frag}"
    );
    assert!(
        !frag.contains("colors-dark") && !frag.contains("colors-light"),
        "legacy keeps the refused sections:\n{frag}"
    );
    assert!(frag.contains("alpha=1.00"), "alpha lost:\n{frag}");
    assert!(frag.contains("regular0=1a2230"), "ansi order lost:\n{frag}");
    assert!(frag.contains("bright7=f6eedc"), "bright hues lost:\n{frag}");
    assert!(
        frag.lines()
            .filter(|l| !l.starts_with('#'))
            .all(|l| !l.contains('#')),
        "a `#` escapes into foot.ini:\n{frag}"
    );
}

#[test]
fn foot_legacy_refuses_a_hand_built_bad_palette() {
    // The legacy path checks exactly like the themed one: a library
    // caller can hand-build a `ResolvedLook` past the parse gate.
    let mut look = terminal_look();
    look.terminal = Some(vec!["#000000".to_string(); 15]);
    let err = render_foot_legacy(&look).expect_err("fifteen hues render");
    assert_eq!(err.field, "terminal.colors");
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

#[test]
fn fuzzel_fragment() {
    // The launcher's leaves (design section 3.3, `fuzzel-theme.nix`):
    // the same seven roles the inlined CLI flags carry, as a `fuzzel.ini`
    // `[colors]` table in fuzzel's `rrggbbaa` spelling. No `[main]`
    // section: the font, lines and every other knob stay the user's,
    // exactly what the flags leave alone today.
    let frag = render_fuzzel(&moonrise()).expect("renders");
    for line in [
        "[colors]",
        "background=2B3648ff",
        "text=F6EEDCff",
        "prompt=F6EEDCff",
        "match=FFA45Cff",
        "selection=FFA45Cff",
        "selection-text=2B3648ff",
        "border=FFA45Cff",
    ] {
        assert!(frag.contains(line), "missing {line}:\n{frag}");
    }
    assert!(!frag.contains("[main]"), "font section emitted:\n{frag}");
    assert!(
        !frag.contains("launch-prefix"),
        "command key emitted:\n{frag}"
    );
    assert!(!frag.contains("terminal"), "command key emitted:\n{frag}");
    // The fragment reparses to the same pairs: no line in it opens a
    // new section or key.
    let parsed = parse_kv_config(&frag).expect("fragment parses");
    let colors = parsed.get("colors").expect("a colors section");
    assert_eq!(colors.len(), 7, "seven leaves, no more: {colors:?}");
    assert_eq!(
        colors.get("selection").map(String::as_str),
        Some("FFA45Cff")
    );
}

#[test]
fn swaylock_fragment() {
    // The locker's leaves (`idle-home.nix` `lockTheme`): the same
    // fourteen `key=value` lines the store config carries, in swaylock's
    // `rrggbb` spelling without the `#`.
    let frag = render_swaylock(&moonrise()).expect("renders");
    for line in [
        "color=2B3648",
        "inside-color=2B3648",
        "ring-color=FFA45C",
        "ring-ver-color=FFA45C",
        "ring-clear-color=FFA45C",
        "ring-wrong-color=E87F6A",
        "inside-ver-color=FFA45C",
        "inside-clear-color=FFA45C",
        "inside-wrong-color=E87F6A",
        "key-hl-color=FFA45C",
        "bs-hl-color=FFA45C",
        "text-color=F6EEDC",
        "line-color=9C8B95",
        "separator-color=9C8B95",
    ] {
        assert!(frag.contains(line), "missing {line}:\n{frag}");
    }
    let parsed = parse_kv_config(&frag).expect("fragment parses");
    let global = parsed.get("").expect("section-less pairs");
    assert_eq!(global.len(), 14, "fourteen leaves, no more: {global:?}");
}

#[test]
fn leaf_values_never_span_lines() {
    // Design section 3.5: every renderer quotes for its own format on
    // top of the grammar. These formats have no string quoting, so the
    // writer refuses a value that would open a new line; `=`, `[` and a
    // quote pass through literally and reparse to themselves.
    for hostile in ["a\nb", "a\rb", "a\0b"] {
        assert!(
            kv_line("key", hostile).is_err(),
            "{hostile:?} renders: {:?}",
            kv_line("key", hostile).ok()
        );
    }
    for probe in ["a=b", "a[b", "a\"b", "a'b", "a#b"] {
        let line = kv_line("key", probe).expect("probe renders");
        let parsed = parse_kv_config(&format!("[s]\n{line}\n")).expect("reparses");
        assert_eq!(
            parsed
                .get("s")
                .and_then(|s| s.get("key"))
                .map(String::as_str),
            Some(probe),
            "{probe:?} does not round-trip"
        );
    }
    // A hand-built look carrying a non-hex color refuses at render, so a
    // broken config never reaches the app. (The roles render resolved,
    // so the hostile value goes on the role each renderer reads.)
    let mut look = moonrise();
    look.palette.selection = "accent\nselection-text=000000ff".to_string();
    assert!(render_fuzzel(&look).is_err());
    let mut look = moonrise();
    look.palette.border = "border".to_string();
    assert!(render_swaylock(&look).is_err());
}

#[test]
fn greet_fragment_carries_the_shell_contract() {
    // Design section 9: the login-screen fragment holds all thirteen
    // color roles, the UI sans at its app size, opaque, with the look's
    // radius. The minimal look's defaults resolve surface to the
    // background, hover/focus/selection to the accent, border to dim,
    // and accent-fg/selection-text to the background.
    let frag = render_greet(&moonrise()).expect("renders");
    for line in [
        "background = \"#2B3648\"",
        "foreground = \"#F6EEDC\"",
        "dim = \"#9C8B95\"",
        "accent = \"#FFA45C\"",
        "urgent = \"#E87F6A\"",
        "surface = \"#2B3648\"",
        "hover = \"#FFA45C\"",
        "focus = \"#FFA45C\"",
        "border = \"#9C8B95\"",
        "accent-fg = \"#2B3648\"",
        "selection = \"#FFA45C\"",
        "selection-text = \"#2B3648\"",
        "match = \"#FFA45C\"",
        "font = \"DejaVu Sans\"",
        "font-size = 16",
        "opacity = 1.0",
        "radius = 14",
    ] {
        assert!(frag.contains(line), "missing {line}:\n{frag}");
    }
    // One `[colors]`, and the scalar keys stay top-level (before it):
    // keys after a table would land in that table.
    assert_eq!(frag.matches("[colors]").count(), 1, "colors table:\n{frag}");
    let colors_at = frag.find("[colors]").expect("a colors table");
    for key in ["font =", "font-size =", "opacity =", "radius ="] {
        let at = frag.find(key).expect("a scalar key");
        assert!(at < colors_at, "{key} lands under [colors]:\n{frag}");
    }
    let parsed: toml::Table = toml::from_str(&frag).expect("greet fragment reparses");
    assert_eq!(parsed["colors"].as_table().expect("colors").len(), 13);
    assert_eq!(parsed["font"].as_str(), Some("DejaVu Sans"));
}

#[test]
fn greet_fragment_stays_opaque() {
    // Design section 9: `opacity.popups` reaches scootnotify and
    // scootlaunch, never the greeter. A look translucent elsewhere
    // still renders an opaque login screen.
    let text = crate::schema::tests::minimal_toml().replacen(
        "[shape]",
        "[opacity]\nbar = 0.5\npopups = 0.5\nterminal = 0.5\n[shape]",
        1,
    );
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::User))]);
    let look = resolve(&map, "x").expect("resolves");
    let frag = render_greet(&look).expect("renders");
    assert!(
        frag.contains("opacity = 1.0"),
        "greeter follows popups opacity:\n{frag}"
    );
}

#[test]
fn greet_overrides_merge() {
    // `[apps.scootgreet]` roles replace the derived ones and dotless
    // keys replace the top-level values, each emitted once.
    let text = crate::schema::tests::minimal_toml().replacen(
        "[cursor]",
        "[apps.scootgreet]\n\"colors.accent\" = \"#000000\"\n\"radius\" = 3\n[cursor]",
        1,
    );
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::User))]);
    let look = resolve(&map, "x").expect("resolves");
    let frag = render_greet(&look).expect("renders");
    assert_eq!(
        frag.matches("[colors]").count(),
        1,
        "duplicate table:\n{frag}"
    );
    let parsed: toml::Table = toml::from_str(&frag).expect("greet fragment reparses");
    assert_eq!(parsed["colors"]["accent"].as_str(), Some("#000000"));
    assert_eq!(parsed["radius"].as_integer(), Some(3));
    // A scoot key through the greeter renderer is refused, not misplaced.
    let mut look = moonrise();
    look.apps = BTreeMap::from([(
        "scootgreet".to_string(),
        BTreeMap::from([("binds.foo".to_string(), toml::Value::String("x".into()))]),
    )]);
    assert!(render_greet(&look).is_err());
}

#[test]
fn launch_fragment_carries_the_shell_contract() {
    // The launcher fragment holds all thirteen color roles, the UI font
    // at its popup size, translucent at the popups opacity, with the
    // look's radius. Moonrise sets no popups opacity, so it renders the
    // 1.0 default; its popup size defaults to its 10 pt size (13 px).
    let frag = render_launch(&moonrise(), &RenderInputs::default()).expect("renders");
    for line in [
        "background = \"#2B3648\"",
        "foreground = \"#F6EEDC\"",
        "dim = \"#9C8B95\"",
        "accent = \"#FFA45C\"",
        "urgent = \"#E87F6A\"",
        "surface = \"#2B3648\"",
        "hover = \"#FFA45C\"",
        "focus = \"#FFA45C\"",
        "border = \"#9C8B95\"",
        "accent-fg = \"#2B3648\"",
        "selection = \"#FFA45C\"",
        "selection-text = \"#2B3648\"",
        "match = \"#FFA45C\"",
        "font = \"DroidSansM Nerd Font Propo\"",
        "font-size = 13",
        "opacity = 1.0",
        "radius = 14",
        "theme = \"Adwaita\"",
    ] {
        assert!(frag.contains(line), "missing {line}:\n{frag}");
    }
    // One `[colors]`, one `[icons]`, and the scalar keys stay top-level
    // (before the first table): keys after a table would land in it.
    assert_eq!(frag.matches("[colors]").count(), 1, "colors table:\n{frag}");
    assert_eq!(frag.matches("[icons]").count(), 1, "icons table:\n{frag}");
    let colors_at = frag.find("[colors]").expect("a colors table");
    for key in ["font =", "font-size =", "opacity =", "radius ="] {
        let at = frag.find(key).expect("a scalar key");
        assert!(at < colors_at, "{key} lands under [colors]:\n{frag}");
    }
    let parsed: toml::Table = toml::from_str(&frag).expect("launch fragment reparses");
    assert_eq!(parsed["colors"].as_table().expect("colors").len(), 13);
    assert_eq!(parsed["icons"]["theme"].as_str(), Some("Adwaita"));
    assert_eq!(parsed["font"].as_str(), Some("DroidSansM Nerd Font Propo"));
}

#[test]
fn launch_fragment_follows_popups_opacity() {
    // Unlike the greeter's, the launcher's opacity is the look's
    // `opacity.popups`: a translucent look draws a translucent panel.
    let text = crate::schema::tests::minimal_toml().replacen(
        "[shape]",
        "[opacity]\nbar = 0.5\npopups = 0.5\nterminal = 0.5\n[shape]",
        1,
    );
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::User))]);
    let look = resolve(&map, "x").expect("resolves");
    let frag = render_launch(&look, &RenderInputs::default()).expect("renders");
    assert!(
        frag.contains("opacity = 0.5"),
        "launcher follows popups opacity:\n{frag}"
    );
}

#[test]
fn launch_overrides_merge() {
    // `[apps.scootlaunch]` roles replace the derived ones and dotless
    // keys replace the top-level values, each emitted once.
    let text = crate::schema::tests::minimal_toml().replacen(
        "[cursor]",
        "[apps.scootlaunch]\n\"colors.accent\" = \"#000000\"\n\"radius\" = 3\n[cursor]",
        1,
    );
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::User))]);
    let look = resolve(&map, "x").expect("resolves");
    let frag = render_launch(&look, &RenderInputs::default()).expect("renders");
    assert_eq!(
        frag.matches("[colors]").count(),
        1,
        "duplicate table:\n{frag}"
    );
    let parsed: toml::Table = toml::from_str(&frag).expect("launch fragment reparses");
    assert_eq!(parsed["colors"]["accent"].as_str(), Some("#000000"));
    assert_eq!(parsed["radius"].as_integer(), Some(3));
    // A scoot key through the launcher renderer is refused, not misplaced.
    let mut look = moonrise();
    look.apps = BTreeMap::from([(
        "scootlaunch".to_string(),
        BTreeMap::from([("binds.foo".to_string(), toml::Value::String("x".into()))]),
    )]);
    assert!(render_launch(&look, &RenderInputs::default()).is_err());
}

#[test]
fn notify_fragment_carries_the_shell_contract() {
    // The notification fragment holds the same thirteen color roles,
    // the UI font at its popup size, translucent at the popups opacity,
    // with the look's radius: whatever the launcher draws, the stack
    // draws, key for key.
    let launch = render_launch(&moonrise(), &RenderInputs::default()).expect("renders");
    let frag = render_notify(&moonrise(), &RenderInputs::default()).expect("renders");
    for line in launch.lines() {
        assert!(
            frag.contains(line),
            "notify misses launch line {line:?}:\n{frag}"
        );
    }
    let parsed: toml::Table = toml::from_str(&frag).expect("notify fragment reparses");
    assert_eq!(parsed["colors"].as_table().expect("colors").len(), 13);
    assert_eq!(parsed["icons"]["theme"].as_str(), Some("Adwaita"));
}

#[test]
fn notify_overrides_merge_under_its_own_app() {
    // `[apps.scootnotify]` roles replace the derived ones; a
    // `[apps.scootlaunch]` key does not leak into the notify fragment.
    let text = crate::schema::tests::minimal_toml().replacen(
        "[cursor]",
        "[apps.scootnotify]\n\"colors.accent\" = \"#000000\"\n\"radius\" = 3\n[cursor]",
        1,
    );
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::User))]);
    let look = resolve(&map, "x").expect("resolves");
    let frag = render_notify(&look, &RenderInputs::default()).expect("renders");
    let parsed: toml::Table = toml::from_str(&frag).expect("notify fragment reparses");
    assert_eq!(parsed["colors"]["accent"].as_str(), Some("#000000"));
    assert_eq!(parsed["radius"].as_integer(), Some(3));
    // A scoot key through the notify renderer is refused, not misplaced.
    let mut look = moonrise();
    look.apps = BTreeMap::from([(
        "scootnotify".to_string(),
        BTreeMap::from([("binds.foo".to_string(), toml::Value::String("x".into()))]),
    )]);
    assert!(render_notify(&look, &RenderInputs::default()).is_err());
}

fn with_effects(extra: &str) -> ResolvedLook {
    let text =
        crate::schema::tests::minimal_toml().replacen("[cursor]", &format!("{extra}[cursor]"), 1);
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("x".to_string(), (raw, Origin::User))]);
    resolve(&map, "x").expect("resolves")
}

#[test]
fn a_look_without_effects_renders_the_fragments_it_always_did() {
    // No `[effects]`: no depth key anywhere, so every fragment is
    // byte-identical to what the renderer wrote before the tokens existed.
    let frag = render_notify(&moonrise(), &RenderInputs::default()).expect("renders");
    assert!(!frag.contains("shadow") && !frag.contains("fill"), "{frag}");
}

#[test]
fn effects_reach_the_notifier_and_launcher_fragments_and_no_other() {
    let look = with_effects("[effects]\nshadow = \"soft\"\nfill = \"gradient\"\n");
    let notify: toml::Table =
        toml::from_str(&render_notify(&look, &RenderInputs::default()).expect("renders"))
            .expect("TOML");
    assert_eq!(notify["shadow"].as_str(), Some("soft"));
    assert_eq!(notify["fill"].as_str(), Some("gradient"));
    // The keys are top-level, before any table, like `radius`.
    let text = render_notify(&look, &RenderInputs::default()).expect("renders");
    assert!(
        text.find("shadow").unwrap() < text.find("[colors]").unwrap(),
        "{text}"
    );
    // The launcher reads both; the greeter reads neither yet and would
    // report them as unknown on every load.
    let launch: toml::Table =
        toml::from_str(&render_launch(&look, &RenderInputs::default()).expect("renders"))
            .expect("TOML");
    assert_eq!(launch["shadow"].as_str(), Some("soft"));
    assert_eq!(launch["fill"].as_str(), Some("gradient"));
    let greet = render_greet(&look).expect("renders");
    assert!(
        !greet.contains("shadow") && !greet.contains("fill"),
        "{greet}"
    );
}

#[test]
fn a_launcher_override_beats_the_shared_effects_and_the_notifiers_does_not_leak() {
    let look = with_effects(
        "[effects]\nshadow = \"soft\"\nmotion = \"spring\"\nmotion-duration-ms = 380\n\
         [apps.scootlaunch]\n\"shadow\" = \"glow\"\n\"motion-duration-ms\" = 160\n\
         [apps.scootnotify]\n\"shadow\" = \"deep\"\n",
    );
    let launch: toml::Table =
        toml::from_str(&render_launch(&look, &RenderInputs::default()).expect("renders"))
            .expect("each key once");
    assert_eq!(launch["shadow"].as_str(), Some("glow"));
    assert_eq!(launch["motion"].as_str(), Some("spring"));
    assert_eq!(launch["motion-duration-ms"].as_integer(), Some(160));
    let notify: toml::Table =
        toml::from_str(&render_notify(&look, &RenderInputs::default()).expect("renders"))
            .expect("each key once");
    assert_eq!(notify["shadow"].as_str(), Some("deep"));
    assert_eq!(notify["motion-duration-ms"].as_integer(), Some(380));
}

#[test]
fn a_look_without_effects_renders_the_launcher_fragment_it_always_did() {
    let frag = render_launch(&moonrise(), &RenderInputs::default()).expect("renders");
    for key in ["shadow", "fill", "motion"] {
        assert!(!frag.contains(key), "{key}: {frag}");
    }
}

#[test]
fn the_launcher_refuses_the_notifiers_card_keys() {
    // Not allowlisted for the launcher, so a look carrying one is refused
    // at parse, never rendered into `launch.toml`.
    let text = crate::schema::tests::minimal_toml().replacen(
        "[cursor]",
        "[apps.scootlaunch]\n\"card.mark\" = \"stripe\"\n[cursor]",
        1,
    );
    assert!(parse_look_toml(text.as_bytes()).is_err());
}

#[test]
fn a_notifier_override_beats_the_shared_effects() {
    let look = with_effects(
        "[effects]\nshadow = \"soft\"\n[apps.scootnotify]\n\"shadow\" = \"glow\"\n\"fill\" = \"gradient\"\n",
    );
    let text = render_notify(&look, &RenderInputs::default()).expect("renders");
    let parsed: toml::Table = toml::from_str(&text).expect("each key is emitted once");
    assert_eq!(parsed["shadow"].as_str(), Some("glow"));
    assert_eq!(parsed["fill"].as_str(), Some("gradient"));
}

#[test]
fn a_hand_built_look_with_a_bad_effect_value_is_refused_not_rendered() {
    let mut look = moonrise();
    look.effects.shadow = Some("soft\"\n[autostart]".to_string());
    assert!(render_notify(&look, &RenderInputs::default()).is_err());
    look.effects.shadow = None;
    look.effects.fill = Some("rainbow".to_string());
    assert!(render_notify(&look, &RenderInputs::default()).is_err());
    // The launcher reads the keys, so it refuses the same bad values.
    assert!(render_launch(&look, &RenderInputs::default()).is_err());
    // An app that never reads the keys does not look at them.
    assert!(render_greet(&look).is_ok());
}

#[test]
fn a_look_without_motion_renders_no_motion_key() {
    let frag = render_notify(&moonrise(), &RenderInputs::default()).expect("renders");
    for key in ["motion", "card"] {
        assert!(!frag.contains(key), "{key}: {frag}");
    }
}

#[test]
fn motion_reaches_the_notifier_and_launcher_fragments_and_no_other() {
    let look = with_effects(
        "[effects]\nmotion = \"spring\"\nmotion-duration-ms = 380\nmotion-bounce = 0.12\n",
    );
    let text = render_notify(&look, &RenderInputs::default()).expect("renders");
    let notify: toml::Table = toml::from_str(&text).expect("TOML");
    assert_eq!(notify["motion"].as_str(), Some("spring"));
    assert_eq!(notify["motion-duration-ms"].as_integer(), Some(380));
    assert_eq!(notify["motion-bounce"].as_float(), Some(0.12));
    // Top-level keys come before any table, like `radius`.
    assert!(
        text.find("motion-bounce").unwrap() < text.find("[colors]").unwrap(),
        "{text}"
    );
    let launch: toml::Table =
        toml::from_str(&render_launch(&look, &RenderInputs::default()).expect("renders"))
            .expect("TOML");
    assert_eq!(launch["motion"].as_str(), Some("spring"));
    assert_eq!(launch["motion-duration-ms"].as_integer(), Some(380));
    assert_eq!(launch["motion-bounce"].as_float(), Some(0.12));
    assert!(launch.get("card").is_none());
    let greet = render_greet(&look).expect("renders");
    assert!(!greet.contains("motion"), "{greet}");
}

#[test]
fn card_overrides_render_as_a_card_table_and_beat_the_shared_motion() {
    let look = with_effects(
        "[effects]\nmotion = \"ease-out\"\n[apps.scootnotify]\n\"motion\" = \"spring\"\n\"card.enter\" = \"drop\"\n\"card.exit\" = \"fade\"\n",
    );
    let text = render_notify(&look, &RenderInputs::default()).expect("renders");
    let parsed: toml::Table = toml::from_str(&text).expect("each key is emitted once");
    assert_eq!(parsed["motion"].as_str(), Some("spring"));
    assert_eq!(parsed["card"]["enter"].as_str(), Some("drop"));
    assert_eq!(parsed["card"]["exit"].as_str(), Some("fade"));
}

#[test]
fn a_hand_built_look_with_a_bad_motion_value_is_refused_not_rendered() {
    let mut look = moonrise();
    look.effects.motion = Some("spring\"\n[autostart]".to_string());
    assert!(render_notify(&look, &RenderInputs::default()).is_err());
    look.effects.motion = None;
    look.effects.motion_duration_ms = Some(60_000);
    assert!(render_notify(&look, &RenderInputs::default()).is_err());
    look.effects.motion_duration_ms = None;
    look.effects.motion_bounce = Some(f64::NAN);
    assert!(render_notify(&look, &RenderInputs::default()).is_err());
    look.effects.motion_bounce = Some(7.0);
    assert!(render_notify(&look, &RenderInputs::default()).is_err());
    // The launcher reads the same keys and refuses the same values; an app
    // that never reads them does not look at them.
    assert!(render_launch(&look, &RenderInputs::default()).is_err());
    assert!(render_greet(&look).is_ok());
}

#[test]
fn card_style_and_title_overrides_render_into_their_tables_once() {
    let look = with_effects(
        "[apps.scootnotify]\n\"card.mark\" = \"badge\"\n\"card.shape\" = \"pill\"\n\"card.app-accent\" = true\n\"card.enter\" = \"fade\"\n\"title.weight\" = 600\n\"title.spacing\" = -1\n",
    );
    let text = render_notify(&look, &RenderInputs::default()).expect("renders");
    let parsed: toml::Table = toml::from_str(&text).expect("each key is emitted once");
    assert_eq!(parsed["card"]["mark"].as_str(), Some("badge"));
    assert_eq!(parsed["card"]["shape"].as_str(), Some("pill"));
    assert_eq!(parsed["card"]["app-accent"].as_bool(), Some(true));
    assert_eq!(parsed["card"]["enter"].as_str(), Some("fade"));
    assert_eq!(parsed["title"]["weight"].as_integer(), Some(600));
    assert_eq!(parsed["title"]["spacing"].as_integer(), Some(-1));
}

#[test]
fn a_look_without_card_style_or_title_keys_renders_neither_table() {
    let frag = render_notify(&moonrise(), &RenderInputs::default()).expect("renders");
    for key in [
        "[card]",
        "[title]",
        "mark",
        "app-accent",
        "weight",
        "spacing",
    ] {
        assert!(!frag.contains(key), "{key}: {frag}");
    }
}

#[test]
fn card_style_keys_reach_the_notifier_fragment_and_no_other() {
    let look =
        with_effects("[apps.scootnotify]\n\"card.mark\" = \"stripe\"\n\"title.weight\" = 500\n");
    for other in [
        render_launch(&look, &RenderInputs::default()),
        render_greet(&look),
    ] {
        let other = other.expect("renders");
        assert!(
            !other.contains("stripe") && !other.contains("[title]"),
            "{other}"
        );
    }
}

fn with_font_file(path: &str) -> RenderInputs {
    RenderInputs {
        ui_font_file: Some(path.to_owned()),
        ..RenderInputs::default()
    }
}

#[test]
fn the_resolved_ui_font_rides_launch_and_notify_as_font_file() {
    let inputs = with_font_file("/nix/store/abc-dejavu/share/fonts/DejaVu Sans.ttf");
    for text in [
        render_launch(&moonrise(), &inputs).expect("renders"),
        render_notify(&moonrise(), &inputs).expect("renders"),
    ] {
        let table: toml::Table = toml::from_str(&text).expect("TOML");
        assert_eq!(
            table["font-file"].as_str(),
            Some("/nix/store/abc-dejavu/share/fonts/DejaVu Sans.ttf")
        );
        // The family stays: it names the font for the notes and for a
        // directory walk when the file is gone.
        assert_eq!(table["font"].as_str(), Some("DroidSansM Nerd Font Propo"));
        // A top-level key, before the first table.
        assert!(text.find("font-file").unwrap() < text.find("[colors]").unwrap());
    }
    // The greeter reads the sans, not the UI font: no such key.
    assert!(
        !render_greet(&moonrise())
            .expect("renders")
            .contains("font-file")
    );
}

#[test]
fn no_resolved_font_renders_no_font_file() {
    for text in [
        render_launch(&moonrise(), &RenderInputs::default()).expect("renders"),
        render_notify(&moonrise(), &RenderInputs::default()).expect("renders"),
    ] {
        assert!(!text.contains("font-file"), "{text}");
    }
}

#[test]
fn a_font_file_with_quotes_and_backslashes_is_escaped() {
    let inputs = with_font_file("/odd/\"quoted\"\\path\n/f.ttf");
    let table: toml::Table =
        toml::from_str(&render_launch(&moonrise(), &inputs).expect("renders")).expect("TOML");
    assert_eq!(
        table["font-file"].as_str(),
        Some("/odd/\"quoted\"\\path\n/f.ttf")
    );
}

#[test]
fn an_app_font_override_drops_the_file_resolved_for_the_looks_family() {
    let mut look = moonrise();
    look.apps = BTreeMap::from([(
        "scootlaunch".to_string(),
        BTreeMap::from([("font".to_string(), toml::Value::String("Other Font".into()))]),
    )]);
    let inputs = with_font_file("/fonts/Look.ttf");
    let launch: toml::Table =
        toml::from_str(&render_launch(&look, &inputs).expect("renders")).expect("each key once");
    assert_eq!(launch["font"].as_str(), Some("Other Font"));
    assert!(!launch.contains_key("font-file"), "{launch:?}");
    // The notifier has no override: it keeps the look's file.
    let notify: toml::Table =
        toml::from_str(&render_notify(&look, &inputs).expect("renders")).expect("TOML");
    assert_eq!(notify["font-file"].as_str(), Some("/fonts/Look.ttf"));
}

#[test]
fn an_author_cannot_set_font_file_through_apps() {
    let mut look = moonrise();
    look.apps = BTreeMap::from([(
        "scootnotify".to_string(),
        BTreeMap::from([(
            "font-file".to_string(),
            toml::Value::String("/etc/x".into()),
        )]),
    )]);
    assert!(render_notify(&look, &RenderInputs::default()).is_err());
}
