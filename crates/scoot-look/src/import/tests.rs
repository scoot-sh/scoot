//! Round-trips over real schemes, the Nix-parity check, and malformed
//! input. Fixtures are the published files (values as published at
//! `tinted-theming/schemes` rev `spec-0.11`, whose `LICENSE` is MIT):
//! base16 3024 by Jan T. Sott, base16 Solarized Dark and Light by Ethan
//! Schoonover (modified by aramisgithub), base24 3024 Day by FredHappyface.

use std::collections::BTreeMap;

use super::{ImportOptions, import_base16};
use crate::{Origin, RenderInputs, check_contrast, render_bar, render_scoot, resolve};

// base16 3024 (dark) by Jan T. Sott, as published.
const DARK_3024: &str = r##"
system: "base16"
name: "3024"
author: "Jan T. Sott (http://github.com/idleberg)"
variant: "dark"
palette:
  base00: "#090300" #  ----
  base01: "#3a3432" #  ---
  base02: "#4a4543" #  --
  base03: "#5c5855" #  -
  base04: "#807d7c" #  +
  base05: "#a5a2a2" #  ++
  base06: "#d6d5d4" #  +++
  base07: "#f7f7f7" #  ++++
  base08: "#db2d20" #	red
  base09: "#e8bbd0" #	orange
  base0A: "#fded02" #	yellow
  base0B: "#01a252" #	green
  base0C: "#b5e4f4" #	aqua
  base0D: "#01a0e4" #	blue
  base0E: "#a16a94" #	purple
  base0F: "#cdab53" #	brown
"##;

// base16 Solarized Dark by Ethan Schoonover (modified by aramisgithub).
const DARK_SOLARIZED: &str = r##"
system: "base16"
name: "Solarized Dark"
author: "Ethan Schoonover (modified by aramisgithub)"
variant: "dark"
palette:
  base00: "#002b36"
  base01: "#073642"
  base02: "#586e75"
  base03: "#657b83"
  base04: "#839496"
  base05: "#93a1a1"
  base06: "#eee8d5"
  base07: "#fdf6e3"
  base08: "#dc322f"
  base09: "#cb4b16"
  base0A: "#b58900"
  base0B: "#859900"
  base0C: "#2aa198"
  base0D: "#268bd2"
  base0E: "#6c71c4"
  base0F: "#d33682"
"##;

// base16 Solarized Light by Ethan Schoonover (modified by aramisgithub).
const LIGHT_SOLARIZED: &str = r##"
system: "base16"
name: "Solarized Light"
author: "Ethan Schoonover (modified by aramisgithub)"
variant: "light"
palette:
  base00: "#fdf6e3"
  base01: "#eee8d5"
  base02: "#93a1a1"
  base03: "#839496"
  base04: "#657b83"
  base05: "#586e75"
  base06: "#073642"
  base07: "#002b36"
  base08: "#dc322f"
  base09: "#cb4b16"
  base0A: "#b58900"
  base0B: "#859900"
  base0C: "#2aa198"
  base0D: "#268bd2"
  base0E: "#6c71c4"
  base0F: "#d33682"
"##;

// base24 3024 Day (light) by FredHappyface: the base24 superset.
const LIGHT_3024_DAY: &str = r##"
system: "base24"
name: "3024 Day"
author: "FredHappyface (https://github.com/fredHappyface)"
variant: "light"
palette:
  base00: "#f7f7f7"
  base01: "#090300"
  base02: "#5c5855"
  base03: "#6e6a68"
  base04: "#807d7b"
  base05: "#928f8e"
  base06: "#a5a2a2"
  base07: "#090300"
  base08: "#db2d20"
  base09: "#fded02"
  base0A: "#807d7c"
  base0B: "#01a252"
  base0C: "#b5e4f4"
  base0D: "#01a0e4"
  base0E: "#a16a94"
  base0F: "#6d1610"
  base10: "#3d3a38"
  base11: "#1e1d1c"
  base12: "#e8bbd0"
  base13: "#4a4543"
  base14: "#3a3432"
  base15: "#cdab53"
  base16: "#807d7c"
  base17: "#d6d5d4"
"##;

fn opts() -> ImportOptions {
    ImportOptions::default()
}

/// The mapping equals today's Nix one for the slots it uses:
/// `home.nix:571-573` (ring active `base0D`, inactive `base03`,
/// background `base00`) and `scootbar.nix:482-487` (background `base00`,
/// foreground `base05`, accent and hover `base0D`, dim `base03`, urgent
/// `base08`).
#[test]
fn mapping_matches_todays_nix() {
    let look = import_base16(DARK_3024.as_bytes(), &opts()).expect("imports");
    for (token, slot) in [
        ("background", "#090300"),
        ("foreground", "#a5a2a2"),
        ("dim", "#5c5855"),
        ("accent", "#01a0e4"),
        ("urgent", "#db2d20"),
        ("surface", "#090300"),
        ("hover", "#01a0e4"),
        ("focus", "#01a0e4"),
        ("border", "#5c5855"),
        ("accent-fg", "#090300"),
        ("selection", "#4a4543"),
        ("selection-text", "#a5a2a2"),
        ("match", "#fded02"),
    ] {
        assert!(
            look.contains(&format!("{token} = \"{slot}\"")),
            "{token} maps to {slot}:\n{look}"
        );
    }
    // The terminal hues run in base16's terminal order (ANSI 0-15:
    // base00, base08, base0B, base0A, base0D, base0E, base0C, base05,
    // base03, base08, base0B, base0A, base0D, base0E, base0C, base07),
    // so foot regular0-7 then bright0-7 lands red/green/yellow/blue at
    // ANSI 1-4.
    assert!(look.contains(
        "colors = [\"#090300\", \"#db2d20\", \"#01a252\", \"#fded02\", \"#01a0e4\", \"#a16a94\", \"#b5e4f4\", \"#a5a2a2\", \"#5c5855\", \"#db2d20\", \"#01a252\", \"#fded02\", \"#01a0e4\", \"#a16a94\", \"#b5e4f4\", \"#f7f7f7\"]"
    ));
    // Names slugify: `3024` stays `3024`.
    assert!(look.contains("name = \"3024\""));
}

/// Every real scheme round-trips: import, parse, validate, resolve,
/// contrast (warn-only for a user look), render both fragments.
#[test]
fn round_trip_real_schemes() {
    for (scheme, name, polarity) in [
        (DARK_3024, "3024", "dark"),
        (DARK_SOLARIZED, "solarized-dark", "dark"),
        (LIGHT_SOLARIZED, "solarized-light", "light"),
        (LIGHT_3024_DAY, "3024-day", "light"),
    ] {
        let look = import_base16(scheme.as_bytes(), &opts()).expect("imports");
        assert!(
            look.contains(&format!("name = \"{name}\"")),
            "{name}:\n{look}"
        );
        assert!(
            look.contains(&format!("polarity = \"{polarity}\"")),
            "{name}:\n{look}"
        );
        let raw = crate::parse_look_toml(look.as_bytes()).expect("re-parses");
        assert_eq!(raw.name, name);
        assert!(crate::validate_for_origin(&raw, Origin::User).is_empty());
        assert!(crate::validate_apps_allowlist(&raw.apps).is_empty());
        let map = BTreeMap::from([(name.to_string(), (raw, Origin::User))]);
        let resolved = resolve(&map, name).expect("resolves");
        // A look the user chose warns, never refuses.
        assert!(
            check_contrast(&resolved, Origin::User)
                .refusals()
                .is_empty()
        );
        let inputs = RenderInputs::default();
        let scoot = render_scoot(&resolved, &inputs).expect("renders scoot");
        let bar = render_bar(&resolved, &inputs).expect("renders bar");
        let _: toml::Value = toml::from_str(&scoot).expect("scoot parses");
        let _: toml::Value = toml::from_str(&bar).expect("bar parses");
    }
}

/// Flat files (no `palette:` map, bare hex, old `scheme:` key) import.
#[test]
fn flat_yaml_imports() {
    let flat = "scheme: Flat\nauthor: Tester\nbase00: 090300\nbase01: 3a3432\nbase02: 4a4543\nbase03: 5c5855\nbase04: 807d7c\nbase05: a5a2a2\nbase06: d6d5d4\nbase07: f7f7f7\nbase08: db2d20\nbase09: e8bbd0\nbase0A: fded02\nbase0B: 01a252\nbase0C: b5e4f4\nbase0D: 01A0E4\nbase0E: a16a94\nbase0F: cdab53\n";
    let look = import_base16(flat.as_bytes(), &opts()).expect("imports");
    assert!(look.contains("name = \"flat\""));
    assert!(look.contains("accent = \"#01a0e4\""));
    assert!(look.contains("polarity = \"dark\""));
    crate::parse_look_toml(look.as_bytes()).expect("re-parses");
}

/// The JSON form (what the Nix side serializes) imports, companions
/// included: fonts, sizes, opacity, cursor.
#[test]
fn json_companion_imports() {
    let json = r##"{
      "scheme": "Json",
      "author": "Tester",
      "base00": "#090300", "base01": "#3a3432", "base02": "#4a4543",
      "base03": "#5c5855", "base04": "#807d7c", "base05": "#a5a2a2",
      "base06": "#d6d5d4", "base07": "#f7f7f7", "base08": "#db2d20",
      "base09": "#e8bbd0", "base0A": "#fded02", "base0B": "#01a252",
      "base0C": "#b5e4f4", "base0D": "#01a0e4", "base0E": "#a16a94",
      "base0F": "#cdab53",
      "fonts": {"sansSerif": "Noto Sans", "monospace": "Noto Mono"},
      "sizes": {"desktop": 11, "popups": 10, "applications": 12, "terminal": 10.5},
      "opacity": {"popups": 0.9, "terminal": 0.8},
      "cursor": {"name": "Adwaita", "size": 32}
    }"##;
    let look = import_base16(json.as_bytes(), &opts()).expect("imports");
    for line in [
        "ui = \"Noto Sans\"",
        "sans = \"Noto Sans\"",
        "mono = \"Noto Mono\"",
        "size = 11.0",
        "popup-size = 10.0",
        "app-size = 12.0",
        "terminal-size = 10.5",
        "popups = 0.9",
        "terminal = 0.8",
        "theme = \"Adwaita\"",
        "size = 32",
    ] {
        assert!(look.contains(line), "missing {line}:\n{look}");
    }
    // The bar's opacity is left unset, as today.
    assert!(!look.contains("bar = "));
    crate::parse_look_toml(look.as_bytes()).expect("re-parses");
}

/// With `--extends` the output is a child: palette only (plus companion
/// fonts when given), no terminal, shape or wallpaper.
#[test]
fn extends_omits_terminal_shape_wallpaper() {
    let child_opts = ImportOptions {
        extends: Some("ginger-night".to_string()),
        ..ImportOptions::default()
    };
    let look = import_base16(DARK_SOLARIZED.as_bytes(), &child_opts).expect("imports");
    assert!(look.contains("extends = \"ginger-night\""));
    assert!(look.contains("accent = \"#268bd2\""));
    for section in ["[terminal]", "[shape]", "[wallpaper]"] {
        assert!(!look.contains(section), "child carries {section}:\n{look}");
    }
    let raw = crate::parse_look_toml(look.as_bytes()).expect("re-parses");
    // The child resolves over a base holding the required tokens.
    let base = crate::parse_look_toml(
        import_base16(DARK_3024.as_bytes(), &opts())
            .expect("base imports")
            .as_bytes(),
    )
    .expect("base parses");
    let map = BTreeMap::from([
        ("solarized-dark".to_string(), (raw, Origin::User)),
        ("3024".to_string(), (base, Origin::User)),
    ]);
    // `extends` names `ginger-night`, unknown here: rewrite to the base.
    let mut child = map["solarized-dark"].0.clone();
    child.extends = Some("3024".to_string());
    let map = BTreeMap::from([
        ("solarized-dark".to_string(), (child, Origin::User)),
        ("3024".to_string(), (map["3024"].0.clone(), Origin::User)),
    ]);
    let resolved = resolve(&map, "solarized-dark").expect("child resolves");
    assert_eq!(resolved.palette.accent, "#268bd2");
    assert_eq!(resolved.palette.background, "#002b36");
}

/// An image plus a mode emits a wallpaper unit carrying both, with and
/// without `--extends`; without an image a child inherits the base's
/// and a root keeps the flat color.
#[test]
fn image_emits_wallpaper_unit() {
    let with_image = DARK_3024.replace(
        "variant: \"dark\"",
        "variant: \"dark\"\nimage: wallpaper.png\nimageScalingMode: fill",
    );
    let look = import_base16(with_image.as_bytes(), &opts()).expect("imports");
    assert!(look.contains("image = \"wallpaper.png\""), ":\n{look}");
    assert!(look.contains("mode = \"fill\""), ":\n{look}");
    assert!(
        !look.contains("color = "),
        "flat color beside the image:\n{look}"
    );
    crate::parse_look_toml(look.as_bytes()).expect("re-parses");
    // The mode also spells `mode`.
    let mode_key = DARK_3024.replace(
        "variant: \"dark\"",
        "variant: \"dark\"\nimage: wall.webp\nmode: tile",
    );
    let look = import_base16(mode_key.as_bytes(), &opts()).expect("imports");
    assert!(look.contains("image = \"wall.webp\""), ":\n{look}");
    assert!(look.contains("mode = \"tile\""), ":\n{look}");
    // A child with an image carries its own unit; without one it
    // inherits the base's (no unit at all).
    let child_opts = ImportOptions {
        extends: Some("ginger-night".to_string()),
        ..ImportOptions::default()
    };
    let look = import_base16(with_image.as_bytes(), &child_opts).expect("imports");
    assert!(look.contains("[wallpaper]"), ":\n{look}");
    assert!(look.contains("image = \"wallpaper.png\""), ":\n{look}");
    let plain = import_base16(DARK_SOLARIZED.as_bytes(), &child_opts).expect("imports");
    assert!(!plain.contains("[wallpaper]"), "child inherits:\n{plain}");
    crate::parse_look_toml(look.as_bytes()).expect("re-parses");
}

/// An image without a mode (or the reverse), a bad mode, and a bad file
/// name each refuse naming the field.
#[test]
fn image_needs_mode_and_shape() {
    let no_mode = DARK_3024.replace(
        "variant: \"dark\"",
        "variant: \"dark\"\nimage: wallpaper.png",
    );
    let err = import_base16(no_mode.as_bytes(), &opts()).expect_err("image without mode");
    assert_eq!(err.field, "imageScalingMode", "{err:?}");
    let no_image = DARK_3024.replace(
        "variant: \"dark\"",
        "variant: \"dark\"\nimageScalingMode: fill",
    );
    let err = import_base16(no_image.as_bytes(), &opts()).expect_err("mode without image");
    assert_eq!(err.field, "image", "{err:?}");
    for (input, field) in [
        (
            DARK_3024.replace(
                "variant: \"dark\"",
                "variant: \"dark\"\nimage: wallpaper.png\nimageScalingMode: zoom",
            ),
            "imageScalingMode",
        ),
        (
            DARK_3024.replace(
                "variant: \"dark\"",
                "variant: \"dark\"\nimage: ../evil.png\nimageScalingMode: fill",
            ),
            "image",
        ),
        (
            DARK_3024.replace(
                "variant: \"dark\"",
                "variant: \"dark\"\nimage: wallpaper.svg\nimageScalingMode: fill",
            ),
            "image",
        ),
        (
            DARK_3024.replace(
                "variant: \"dark\"",
                "variant: \"dark\"\nimage: /abs/wall.png\nimageScalingMode: fill",
            ),
            "image",
        ),
    ] {
        let err = import_base16(input.as_bytes(), &opts()).expect_err("refuses");
        assert_eq!(err.field, field, "{err:?} for {input:?}");
    }
}

/// Malformed input refuses naming the field, never panics.
#[test]
fn malformed_refuses_by_name() {
    // Missing base0D.
    let mut missing = String::new();
    for n in 0..16 {
        if n != 0x0D {
            missing.push_str(&format!("base{n:02X}: 000000\n"));
        }
    }
    let err = import_base16(missing.as_bytes(), &opts()).expect_err("missing slot");
    assert_eq!(err.field, "base0D", "{err:?}");
    // Bad hex names the slot.
    let bad = DARK_3024.replace("#01a0e4", "not-a-color");
    let err = import_base16(bad.as_bytes(), &opts()).expect_err("bad hex");
    assert_eq!(err.field, "base0D", "{err:?}");
    // A malformed base24 slot refuses too, even though unused.
    let bad24 = LIGHT_3024_DAY.replace("#3d3a38", "#xyz");
    let err = import_base16(bad24.as_bytes(), &opts()).expect_err("bad base24");
    assert_eq!(err.field, "base10", "{err:?}");
    // Empty, NUL, bad YAML, bad polarity, bad companion, bad names.
    for (input, field) in [
        ("", "scheme"),
        ("base00: 000000\n\0", "scheme"),
        ("base00 000000\n", "scheme"),
        (
            &DARK_3024.replace("variant: \"dark\"", "variant: \"dusk\""),
            "polarity",
        ),
        (
            &DARK_3024.replace("variant: \"dark\"", "sizes:\n  desktop: 2\n"),
            "sizes.desktop",
        ),
        (
            &DARK_3024.replace("variant: \"dark\"", "opacity:\n  popups: 2\n"),
            "opacity.popups",
        ),
    ] {
        let err = import_base16(input.as_bytes(), &opts()).expect_err("refuses");
        assert_eq!(err.field, field, "{err:?} for {input:?}");
    }
    let err = import_base16(
        DARK_3024.as_bytes(),
        &ImportOptions {
            name: Some("Caps!".to_string()),
            ..ImportOptions::default()
        },
    )
    .expect_err("bad name");
    assert_eq!(err.field, "name");
    let err = import_base16(
        DARK_3024.as_bytes(),
        &ImportOptions {
            extends: Some("Caps!".to_string()),
            ..ImportOptions::default()
        },
    )
    .expect_err("bad extends");
    assert_eq!(err.field, "extends");
}

/// Hostile inputs never panic, only refuse or import.
#[test]
fn hostile_input_never_panics() {
    // Deterministic garbage over an alphabet rich in YAML/JSON syntax,
    // quotes, NUL and non-UTF8: every outcome is a clean `Ok` or `Err`,
    // never a panic, and oversized inputs refuse on size.
    let alphabet: &[u8] = b"abcdef0123456789ABCDEF[]{}\"'# \t\n\r:.e+-_\0\xff\x80";
    let mut state = 0x51ab3f09u64;
    let mut rng = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let extends = ImportOptions {
        extends: Some("base".to_string()),
        ..ImportOptions::default()
    };
    for _ in 0..2000 {
        let len = (rng() % 96) as usize;
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            bytes.push(alphabet[(rng() % alphabet.len() as u64) as usize]);
        }
        let _ = import_base16(&bytes, &opts());
        let _ = import_base16(&bytes, &extends);
    }
    // Oversized refuses on size.
    let big = vec![b'a'; 64 * 1024 + 1];
    assert!(import_base16(&big, &opts()).is_err());
}

/// The scheme author's punctuation never fails an import: display text
/// sanitizes to the grammar.
#[test]
fn author_punctuation_sanitizes() {
    let tricky = DARK_3024.replace(
        "author: \"Jan T. Sott (http://github.com/idleberg)\"",
        "author: 'A \"quoted\" [bracketed] =author= \\here\\'",
    );
    let look = import_base16(tricky.as_bytes(), &opts()).expect("imports");
    crate::parse_look_toml(look.as_bytes()).expect("re-parses");
    assert!(!look.contains("\"quoted\""));
}

/// Finding 1: `[terminal]` is ANSI order (foot regular0-7 then bright0-7),
/// which is base16's terminal order, not numeric base00..base0F. Red
/// (base08) lands at ANSI 1, green (base0B) at 2, yellow (base0A) at 3,
/// blue (base0D) at 4.
#[test]
fn terminal_is_ansi_order() {
    let look = import_base16(DARK_3024.as_bytes(), &opts()).expect("imports");
    // 3024: base08 red #db2d20, base0B green #01a252, base0A yellow #fded02,
    // base0D blue #01a0e4.
    let line = look
        .lines()
        .find(|l| l.starts_with("colors = "))
        .expect("terminal colors");
    assert!(
        line.contains("\"#090300\", \"#db2d20\", \"#01a252\", \"#fded02\", \"#01a0e4\""),
        "ANSI 0-4 red/green/yellow/blue:\n{line}"
    );
    assert!(
        line.contains(
            "colors = [\"#090300\", \"#db2d20\", \"#01a252\", \"#fded02\", \"#01a0e4\", \"#a16a94\", \"#b5e4f4\", \"#a5a2a2\", \"#5c5855\", \"#db2d20\", \"#01a252\", \"#fded02\", \"#01a0e4\", \"#a16a94\", \"#b5e4f4\", \"#f7f7f7\"]"
        ),
        "full ANSI order:\n{line}"
    );
}

/// Finding 3: a leading UTF-8 BOM strips before dispatch, in both YAML
/// and JSON (JSON-after-BOM must not fall to the YAML branch).
#[test]
fn bom_strips_yaml_and_json() {
    let mut yaml = vec![0xEF, 0xBB, 0xBF];
    yaml.extend_from_slice(DARK_3024.as_bytes());
    let look = import_base16(&yaml, &opts()).expect("BOM YAML imports");
    assert!(look.contains("name = \"3024\""), "BOM keeps slug:\n{look}");
    let json = r##"{"scheme": "Bom", "author": "t", "base00": "#090300", "base01": "#3a3432", "base02": "#4a4543", "base03": "#5c5855", "base04": "#807d7c", "base05": "#a5a2a2", "base06": "#d6d5d4", "base07": "#f7f7f7", "base08": "#db2d20", "base09": "#e8bbd0", "base0A": "#fded02", "base0B": "#01a252", "base0C": "#b5e4f4", "base0D": "#01a0e4", "base0E": "#a16a94", "base0F": "#cdab53"}"##;
    let mut bom_json = vec![0xEF, 0xBB, 0xBF];
    bom_json.extend_from_slice(json.as_bytes());
    let look = import_base16(&bom_json, &opts()).expect("BOM JSON imports");
    assert!(
        look.contains("name = \"bom\""),
        "BOM JSON keeps slug:\n{look}"
    );
}

/// Finding 5: explicit `title:`/`license:` sanitize like name/author do;
/// generated text never refuses.
#[test]
fn title_license_sanitize_never_refuse() {
    let with_title = DARK_3024.replace(
        "author: \"Jan T. Sott (http://github.com/idleberg)\"",
        "author: \"t\"\ntitle: \"bad\x01title\"",
    );
    let look = import_base16(with_title.as_bytes(), &opts()).expect("title control sanitizes");
    crate::parse_look_toml(look.as_bytes()).expect("re-parses");
    let with_license = DARK_3024.replace(
        "author: \"Jan T. Sott (http://github.com/idleberg)\"",
        "author: \"t\"\nlicense: \"a=b[c]\"",
    );
    let look =
        import_base16(with_license.as_bytes(), &opts()).expect("license punctuation sanitizes");
    crate::parse_look_toml(look.as_bytes()).expect("re-parses");
    assert!(!look.contains("a=b[c]"), "license sanitized:\n{look}");
}

/// Finding 6: bidi controls and line/paragraph separators strip out of
/// description/credits.
#[test]
fn bidi_and_separators_strip() {
    let tricky = DARK_3024.replace(
        "author: \"Jan T. Sott (http://github.com/idleberg)\"",
        "author: \"evil\u{202e} test\"",
    );
    let look = import_base16(tricky.as_bytes(), &opts()).expect("imports");
    assert!(!look.contains('\u{202e}'), "bidi stripped:\n{look}");
    let tricky = DARK_3024.replace(
        "author: \"Jan T. Sott (http://github.com/idleberg)\"",
        "author: \"t\"\ndescription: \"a\u{2028}b\u{2029}c\"",
    );
    let look = import_base16(tricky.as_bytes(), &opts()).expect("imports");
    assert!(!look.contains('\u{2028}'), "line sep stripped:\n{look}");
    assert!(!look.contains('\u{2029}'), "para sep stripped:\n{look}");
    crate::parse_look_toml(look.as_bytes()).expect("re-parses");
}

/// Finding 7: flow collections refuse with a precise error, not a generic
/// hex complaint.
#[test]
fn flow_collection_precise_error() {
    let flow = DARK_3024.replace("base00: \"#090300\"", "base00: {foo: bar}");
    let err = import_base16(flow.as_bytes(), &opts()).expect_err("flow refuses");
    assert!(
        err.reason.contains("flow"),
        "names flow collections, got: {err:?}"
    );
}
