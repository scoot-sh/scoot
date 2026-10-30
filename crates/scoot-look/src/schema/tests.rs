//! Tests for the schema: the moonrise-shaped minimal look parses, and
//! every refusal is named.

use super::*;

const MINIMAL: &str = r##"
schema = 1
name = "moonrise"
title = "Moonrise"
description = "Slate navy."
polarity = "dark"

[palette]
background = "#2B3648"
foreground = "#F6EEDC"
dim = "#9C8B95"
accent = "#FFA45C"
urgent = "#E87F6A"

[fonts]
ui = "DroidSansM Nerd Font Propo"
sans = "DejaVu Sans"
mono = "FiraCode Nerd Font"
size = 10

[shape]
radius = 14
ring-width = 2

[cursor]
theme = "Mocu"
size = 24
accent = "#FF9A49"

[wallpaper]
image = "wallpaper.png"
mode = "fill"
"##;

pub fn minimal_toml() -> String {
    MINIMAL.to_string()
}

#[test]
fn minimal_look_parses() {
    let raw = parse_look_toml(MINIMAL.as_bytes()).expect("minimal look parses");
    assert_eq!(raw.name, "moonrise");
    assert_eq!(raw.palette.background.as_deref(), Some("#2B3648"));
}

#[test]
fn oversized_file_refused() {
    let big = vec![b'#'; MAX_FILE_BYTES + 1];
    let err = parse_look_toml(&big).expect_err("oversized file parses");
    assert!(err.iter().any(|e| e.field == "look.toml"));
}

#[test]
fn file_cap_is_64kib() {
    assert_eq!(MAX_FILE_BYTES, 64 * 1024);
}

#[test]
fn malformed_toml_refused() {
    let err = parse_look_toml(b"schema = ").expect_err("malformed TOML parses");
    assert_eq!(err[0].field, "look.toml");
}

#[test]
fn unknown_schema_refused() {
    let text = MINIMAL.replacen("schema = 1", "schema = 2", 1);
    let err = parse_look_toml(text.as_bytes()).expect_err("schema 2 parses");
    assert!(err.iter().any(|e| e.field == "schema"));
}

#[test]
fn unknown_top_level_key_refused() {
    let text = MINIMAL.replacen("polarity = ", "gap = 8\npolarity = ", 1);
    assert!(parse_look_toml(text.as_bytes()).is_err());
}

#[test]
fn no_gap_in_v1() {
    // `gap` is layout, not presentation (D7): it must not parse as a look key.
    let text = MINIMAL.replacen("[shape]", "[shape]\ngap = 8", 1);
    assert!(parse_look_toml(text.as_bytes()).is_err());
}

#[test]
fn bad_colors_refused() {
    let text = MINIMAL.replacen("#2B3648", "blue", 1);
    let err = parse_look_toml(text.as_bytes()).expect_err("named color parses");
    assert!(err.iter().any(|e| e.field == "palette.background"));
}

#[test]
fn terminal_needs_sixteen() {
    let text = MINIMAL.replacen(
        "[cursor]",
        "[terminal]\ncolors = [\"#000000\"]\n[cursor]",
        1,
    );
    let err = parse_look_toml(text.as_bytes()).expect_err("one terminal color parses");
    assert!(err.iter().any(|e| e.field == "terminal.colors"));
}

#[test]
fn wallpaper_image_and_color_are_exclusive() {
    let text = MINIMAL.replacen(
        "image = \"wallpaper.png\"",
        "image = \"wallpaper.png\"\ncolor = \"#2B3648\"",
        1,
    );
    let err = parse_look_toml(text.as_bytes()).expect_err("image+color parses");
    assert!(err.iter().any(|e| e.field == "wallpaper"));
}

#[test]
fn url_wallpaper_needs_pin() {
    let text = MINIMAL.replacen(
        "image = \"wallpaper.png\"",
        "image = \"https://example.com/w.png\"",
        1,
    );
    let err = parse_look_toml(text.as_bytes()).expect_err("unpinned URL parses");
    assert!(err.iter().any(|e| e.field == "wallpaper.sha256"));
}

#[test]
fn nested_carrier_paths_check_basename() {
    // Regression: the suffix check runs on the basename, so a nested legal
    // path passes and a nested illegal suffix refuses.
    let ok = MINIMAL.replacen(
        "[cursor]",
        "[fonts.files]\nui = \"fonts/DroidSansM-Regular.otf\"\n[cursor]",
        1,
    );
    assert!(parse_look_toml(ok.as_bytes()).is_ok());
    let bad = MINIMAL.replacen(
        "[cursor]",
        "[fonts.files]\nui = \"fonts/evil.so\"\n[cursor]",
        1,
    );
    assert!(parse_look_toml(bad.as_bytes()).is_err());
}

#[test]
fn preview_is_preview_png_and_bundled_fonts_need_a_license() {
    // Finding 6: `preview` took any file name though the design fixes it
    // as `preview.png`, and `[fonts.files]` never required the license the
    // design demands for bundled fonts.
    let bad = MINIMAL.replacen("[palette]", "preview = \"other.png\"\n[palette]", 1);
    let err = parse_look_toml(bad.as_bytes()).expect_err("free-form preview parses");
    assert!(err.iter().any(|e| e.field == "preview"), "{err:?}");
    let ok = MINIMAL.replacen("[palette]", "preview = \"preview.png\"\n[palette]", 1);
    assert!(parse_look_toml(ok.as_bytes()).is_ok());

    let fonts = MINIMAL.replacen(
        "[cursor]",
        "[fonts.files]\nui = \"fonts/DroidSansM-Regular.otf\"\n[cursor]",
        1,
    );
    let raw = parse_look_toml(fonts.as_bytes()).expect("bundled fonts parse");
    for origin in [
        Origin::User,
        Origin::Declared,
        Origin::Shipped,
        Origin::Installed,
    ] {
        assert!(
            validate_for_origin(&raw, origin)
                .iter()
                .any(|e| e.field == "license"),
            "{origin:?} with bundled fonts needs no license"
        );
    }
    let licensed = fonts.replacen("[palette]", "license = \"MIT\"\n[palette]", 1);
    let raw = parse_look_toml(licensed.as_bytes()).expect("licensed parses");
    assert!(validate_for_origin(&raw, Origin::User).is_empty());
}

#[test]
fn license_required_for_shipped_and_installed() {
    let raw = parse_look_toml(MINIMAL.as_bytes()).unwrap();
    assert!(
        validate_for_origin(&raw, Origin::Shipped)
            .iter()
            .any(|e| e.field == "license")
    );
    assert!(
        validate_for_origin(&raw, Origin::Installed)
            .iter()
            .any(|e| e.field == "license")
    );
    assert!(validate_for_origin(&raw, Origin::User).is_empty());
    assert!(validate_for_origin(&raw, Origin::Declared).is_empty());
}

#[test]
fn apps_allowlist_is_enforced_by_parse() {
    // Finding 3: `parse_look_toml` accepted any app and key, with
    // enforcement in a separate function a library caller could skip,
    // while the site page claimed only allowlisted keys parse.
    let bad = MINIMAL.replacen(
        "[cursor]",
        "[apps.scoot]\n\"binds.foo\" = \"x\"\n[cursor]",
        1,
    );
    let err = parse_look_toml(bad.as_bytes()).expect_err("non-allowlisted key parses");
    assert!(
        err.iter().any(|e| e.reason.contains("allowlist")),
        "unnamed allowlist refusal: {err:?}"
    );
    let bad = MINIMAL.replacen(
        "[cursor]",
        "[apps.evil]\n\"colors.accent\" = \"#FFA45C\"\n[cursor]",
        1,
    );
    assert!(parse_look_toml(bad.as_bytes()).is_err());
}

#[test]
fn app_values_are_checked_per_key() {
    // Finding 3: values got only a control-char and 512-byte check, so
    // `nothex` colors and negative sizes parsed and rendered.
    for (app, key, value) in [
        ("scootbar", "colors.accent", "\"nothex\""),
        ("scootbar", "bar.radius", "-5"),
        ("scootbar", "bar.font-size", "0"),
        ("scootbar", "bar.opacity", "1.5"),
        ("scootbar", "workspaces.display", "\"grid\""),
        ("scootbar", "window-title.icon", "\"a=b\""),
        ("scoot", "appearance.background_color", "\"nothex\""),
        ("scoot", "appearance.cursor_size", "3"),
        ("scoot", "wallpaper.mode", "\"spin\""),
        ("scootnotify", "colors.accent", "\"nothex\""),
    ] {
        let bad = MINIMAL.replacen(
            "[cursor]",
            &format!("[apps.{app}]\n\"{key}\" = {value}\n[cursor]"),
            1,
        );
        assert!(
            parse_look_toml(bad.as_bytes()).is_err(),
            "{app}[{key}] = {value} parses"
        );
    }
    // The valid forms of the same keys still parse.
    for (app, key, value) in [
        ("scootbar", "colors.accent", "\"#000000\""),
        ("scootbar", "bar.radius", "3"),
        ("scootbar", "bar.font-size", "13"),
        ("scootbar", "bar.opacity", "0.8"),
        ("scootbar", "workspaces.display", "\"dots\""),
        ("scootbar", "window-title.icon", "\"●\""),
        ("scoot", "appearance.background_color", "\"#000000\""),
        ("scoot", "appearance.cursor_size", "24"),
        ("scoot", "wallpaper.mode", "\"tile\""),
        ("scootnotify", "colors.accent", "\"#000000\""),
    ] {
        let ok = MINIMAL.replacen(
            "[cursor]",
            &format!("[apps.{app}]\n\"{key}\" = {value}\n[cursor]"),
            1,
        );
        assert!(
            parse_look_toml(ok.as_bytes()).is_ok(),
            "{app}[{key}] = {value} refused"
        );
    }
}

#[test]
fn name_must_match_the_directory() {
    // Finding 6: the site page says `name` matches the directory name, but
    // nothing checked it.
    let raw = parse_look_toml(MINIMAL.as_bytes()).expect("minimal parses");
    assert!(check_name_matches_dir(&raw, "moonrise").is_none());
    let err = check_name_matches_dir(&raw, "other").expect("mismatch passes");
    assert_eq!(err.field, "name");
}

#[test]
fn wallpaper_image_must_be_an_image() {
    // Round 2 finding 7: `wallpaper.image = "look.toml"` passed.
    for bad in [
        "look.toml",
        "README.md",
        "LICENSE-MIT",
        "font.ttf",
        "notes.txt",
    ] {
        let text = MINIMAL.replacen(
            "image = \"wallpaper.png\"",
            &format!("image = \"{bad}\""),
            1,
        );
        let err = parse_look_toml(text.as_bytes())
            .expect_err(&format!("{bad} parses as wallpaper image"));
        assert!(
            err.iter().any(|e| e.field == "wallpaper.image"),
            "wrong field for {bad}: {err:?}"
        );
    }
}

#[test]
fn scootbar_font_keys_refuse_at_parse() {
    // Round 2 finding 3: `bar.font` / `bar.fallback-fonts` are file
    // paths the tool resolves, never overrides an author writes.
    for key_value in [
        "\"bar.font\" = \"DejaVu Sans\"",
        "\"bar.fallback-fonts\" = [\"DejaVu Sans\"]",
    ] {
        let text = MINIMAL.replacen(
            "[cursor]",
            &format!("[apps.scootbar]\n{key_value}\n[cursor]"),
            1,
        );
        assert!(
            parse_look_toml(text.as_bytes()).is_err(),
            "{key_value} parses"
        );
    }
}

#[test]
fn override_image_leaf_refuses_at_parse() {
    // Round 2 finding 4: per-output/workspace `image` overrides have
    // no resolved absolute to render, so they refuse loudly instead of
    // rendering a relative path that yields no wallpaper.
    for key in [
        "wallpaper.output.DP-1.image",
        "wallpaper.workspace.2.image",
        "wallpaper.output.DP-1.workspace.2.image",
    ] {
        let text = MINIMAL.replacen(
            "[cursor]",
            &format!("[apps.scoot]\n\"{key}\" = \"a.png\"\n[cursor]"),
            1,
        );
        assert!(parse_look_toml(text.as_bytes()).is_err(), "{key} parses");
    }
}

#[test]
fn nested_workspace_refuses_at_parse() {
    // Round 2 finding 6.
    let text = MINIMAL.replacen(
        "[cursor]",
        "[apps.scoot]\n\"wallpaper.workspace.a.workspace.b.image\" = \"a.png\"\n[cursor]",
        1,
    );
    assert!(parse_look_toml(text.as_bytes()).is_err());
}

#[test]
fn i32_overflow_override_refuses_at_parse() {
    // Round 2 finding 5 at the parse gate: scoot reads
    // `appearance.corner_radius` as an `i32`
    // (`crates/scoot/src/compositor/config.rs:174`), so an `i64`
    // past its range would discard the whole file there.
    let text = MINIMAL.replacen(
        "[cursor]",
        "[apps.scoot]\n\"appearance.corner_radius\" = 9223372036854775807\n[cursor]",
        1,
    );
    assert!(parse_look_toml(text.as_bytes()).is_err());
}

#[test]
fn effects_parse_and_default_to_unset() {
    let none = parse_look_toml(minimal_toml().as_bytes()).expect("parses");
    assert!(none.effects.shadow.is_none() && none.effects.fill.is_none());
    let text = minimal_toml().replacen(
        "[cursor]",
        "[effects]\nshadow = \"deep\"\nfill = \"gradient\"\n[cursor]",
        1,
    );
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    assert_eq!(raw.effects.shadow.as_deref(), Some("deep"));
    assert_eq!(raw.effects.fill.as_deref(), Some("gradient"));
}

#[test]
fn effects_refuse_an_unknown_value_or_key_by_name() {
    for (extra, field) in [
        ("shadow = \"huge\"", "effects.shadow"),
        ("fill = \"rainbow\"", "effects.fill"),
        ("shadow = \"Soft\"", "effects.shadow"),
    ] {
        let text = minimal_toml().replacen("[cursor]", &format!("[effects]\n{extra}\n[cursor]"), 1);
        let errors = parse_look_toml(text.as_bytes()).expect_err("refused");
        assert!(
            errors.iter().any(|e| e.to_string().contains(field)),
            "{extra}: {errors:?}"
        );
    }
    let text = minimal_toml().replacen("[cursor]", "[effects]\nblur = 3\n[cursor]", 1);
    assert!(
        parse_look_toml(text.as_bytes()).is_err(),
        "an unknown [effects] key refuses"
    );
}
