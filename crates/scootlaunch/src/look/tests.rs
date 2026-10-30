//! The look layer's tests: defaults, the merge, refusals, and the
//! per-shipped-look goldens against today's fuzzel colors.

use super::*;

#[test]
fn garbage_is_defaults_with_a_note() {
    let (look, notes) = parse("not toml [[[\n");
    assert_eq!(look, Look::default());
    assert_eq!(notes.len(), 1);
}

#[test]
fn an_empty_fragment_is_the_defaults() {
    let (look, notes) = parse("");
    assert_eq!(look, Look::default());
    assert!(notes.is_empty());
}

#[test]
fn every_key_lands() {
    let (look, notes) = parse(
        "font = \"DejaVu Sans\"\nfont-size = 15\nopacity = 0.8\nradius = 4\n\
         [colors]\nbackground = \"#101010\"\nforeground = \"#f0f0f0\"\ndim = \"#505050\"\n\
         accent = \"#ff0000\"\nurgent = \"#00ff00\"\nsurface = \"#202020\"\n\
         hover = \"#0000ff\"\nfocus = \"#ffff00\"\nborder = \"#123456\"\n\
         accent-fg = \"#654321\"\nselection = \"#00ffff\"\n\
         selection-text = \"#ff00ff\"\nmatch = \"#f0f000\"\n\
         [icons]\ntheme = \"Papirus\"\n",
    );
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(look.theme.background, Color::parse("#ff00ff").unwrap());
    assert_eq!(look.theme.foreground, Color::parse("#f0f0f0").unwrap());
    assert_eq!(look.theme.dim, Color::parse("#505050").unwrap());
    assert_eq!(look.theme.accent, Color::parse("#00ffff").unwrap());
    assert_eq!(look.theme.urgent, Color::parse("#00ff00").unwrap());
    assert_eq!(look.theme.hover, Color::parse("#f0f000").unwrap());
    assert_eq!(look.surface, Color::parse("#202020").unwrap());
    assert_eq!(look.focus, Color::parse("#ffff00").unwrap());
    assert_eq!(look.font_size_px, 15.0);
    assert_eq!(look.opacity, 0.8);
    assert_eq!(look.radius, 4);
    assert_eq!(look.font_family, "DejaVu Sans");
    assert_eq!(look.icon_theme, "Papirus");
}

#[test]
fn refused_keys_keep_defaults_and_are_named() {
    // `not-a-key` rides top-level (TOML keys precede the first table);
    // the `[other]` table is one refusal for the whole table.
    let (look, notes) = parse(
        "not-a-key = 1\n[colors]\nbackground = \"red\"\nforeground = \"#gggggg\"\n\
         [other]\nbackground = \"#101010\"\n",
    );
    assert_eq!(look.theme.background, Look::default().theme.background);
    assert_eq!(look.theme.foreground, Look::default().theme.foreground);
    assert_eq!(notes.len(), 4, "{notes:?}");
    assert!(notes.iter().any(|note| note.contains("not-a-key")));
    assert!(notes.iter().any(|note| note.contains("colors.background")));
    assert!(notes.iter().any(|note| note.contains("colors.foreground")));
    assert!(notes.iter().any(|note| note.contains("`other`")));
}

#[test]
fn hostile_input_never_panics() {
    for text in [
        "[colors]\nbackground = [1, 2]\n",
        "[colors]\nbackground = 1.5\n",
        "font-size = \"big\"\n",
        "opacity = \"opaque\"\n",
        "radius = -1\n",
        "font = [\"x\"]\n",
        "font = \"a\u{0}b\"\n",
        &"x".repeat(70 * 1024),
    ] {
        let (look, notes) = parse(text);
        assert!(!notes.is_empty(), "{text:?}");
        let _ = look;
    }
}

#[test]
fn the_depth_and_motion_keys_land_and_a_default_look_asks_for_none() {
    let (look, notes) = parse(
        "shadow = \"deep\"\nfill = \"gradient\"\nmotion = \"spring\"\n\
         motion-duration-ms = 220\nmotion-bounce = 0.5\n",
    );
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(look.shadow, ShadowStyle::Deep);
    assert_eq!(look.fill, FillStyle::Gradient);
    assert_eq!(look.motion, Curve::Spring);
    assert_eq!(look.motion_ms, 220);
    assert_eq!(look.motion_bounce, 0.5);
    assert!(look.timing().active());
    assert_eq!(look.timing().duration().as_millis(), 220);
    // Nothing set: no shadow, a solid fill, no motion.
    let defaults = Look::default();
    assert_eq!(defaults.shadow, ShadowStyle::None);
    assert_eq!(defaults.fill_stops(), (defaults.surface, defaults.surface));
    assert!(!defaults.timing().active());
    // A duration or a curve alone does not start anything: `motion` does.
    let (look, _) = parse("motion-duration-ms = 300\n");
    assert!(!look.timing().active());
    let (look, _) = parse("motion = \"ease-out\"\nmotion-duration-ms = 0\n");
    assert!(!look.timing().active(), "a zero duration is no motion");
    let (look, _) = parse("motion = \"ease-out\"\n");
    assert_eq!(
        look.timing().duration().as_millis(),
        u128::from(DEFAULT_MOTION_MS)
    );
}

#[test]
fn bad_depth_and_motion_values_are_noted_and_ignored() {
    for (text, key) in [
        ("shadow = \"huge\"\n", "shadow"),
        ("shadow = 3\n", "shadow"),
        ("fill = \"rainbow\"\n", "fill"),
        ("motion = \"bouncy\"\n", "motion"),
        ("motion-duration-ms = 5000\n", "motion-duration-ms"),
        ("motion-duration-ms = -1\n", "motion-duration-ms"),
        ("motion-duration-ms = \"fast\"\n", "motion-duration-ms"),
        ("motion-bounce = 2.0\n", "motion-bounce"),
        ("motion-bounce = \"lots\"\n", "motion-bounce"),
    ] {
        let (look, notes) = parse(text);
        assert_eq!(look, Look::default(), "{text:?}");
        assert_eq!(notes.len(), 1, "{text:?}: {notes:?}");
        assert!(notes[0].contains(key), "{notes:?}");
    }
}

#[test]
fn the_notifiers_card_keys_are_refused_here() {
    let (look, notes) = parse("card = { mark = \"stripe\" }\n[title]\nweight = 600\n");
    assert_eq!(look, Look::default());
    assert_eq!(notes.len(), 2, "{notes:?}");
}

#[test]
fn a_gradient_fill_lights_the_top_of_a_dark_panel() {
    let (look, _) = parse("fill = \"gradient\"\n[colors]\nsurface = \"#1e1e2e\"\n");
    let (top, bottom) = look.fill_stops();
    assert_eq!(bottom, look.surface);
    assert!(top.luminance() > look.surface.luminance());
}

// The goldens: every shipped look draws what fuzzel draws for it today.
// Each row pairs the launch fragment's role with the fuzzel fragment's
// key for the same element (dev/design/scootlaunch.md, Stylix and look
// integration). The fuzzel colors are `rrggbbaa`; only the `rrggbb` half
// is compared (the panel blends its own alpha from `opacity.popups`).

#[cfg(test)]
mod goldens {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use scootui::color::Color;

    const SHIPPED: &[&str] = &[
        "ginger-night",
        "moonrise",
        "music-desk",
        "radial-burst",
        "vinyl-sunset",
    ];

    fn looks_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("looks")
    }

    fn resolved(name: &str) -> scoot_look::ResolvedLook {
        let path = looks_dir().join(name).join("look.toml");
        let bytes =
            std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let raw = scoot_look::parse_look_toml(&bytes).unwrap_or_else(|errors| {
            panic!(
                "{name}/look.toml refuses to parse: {}",
                errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
        let map = BTreeMap::from([(name.to_string(), (raw, scoot_look::Origin::Shipped))]);
        scoot_look::resolve(&map, name).unwrap_or_else(|e| panic!("{name}: does not resolve: {e}"))
    }

    /// A `[colors]` table out of a rendered fuzzel fragment (INI
    /// `key=rrggbbaa` lines, not TOML: parsed with the leaf parser).
    fn fuzzel_colors(fragment: &str) -> BTreeMap<String, String> {
        scoot_look::parse_kv_config(fragment).expect("renders as a kv fragment")["colors"].clone()
    }

    /// A `[colors]` table out of a rendered launch fragment (TOML).
    fn launch_colors(fragment: &str) -> BTreeMap<String, String> {
        let table: toml::Table = fragment.parse().expect("renders as TOML");
        table
            .get("colors")
            .and_then(|colors| colors.as_table())
            .expect("has a [colors] table")
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    value.as_str().expect("a color string").to_owned(),
                )
            })
            .collect()
    }

    /// The `rrggbb` head of a `#rrggbb` launch color or an `rrggbbaa`
    /// fuzzel color, as a `Color`.
    fn rgb(color: &str) -> Color {
        let head = color.strip_prefix('#').unwrap_or(color);
        Color::parse(&format!("#{}", &head[..6])).expect("a rrggbb head")
    }

    #[test]
    fn every_shipped_look_matches_fuzzel() {
        for name in SHIPPED {
            let look = resolved(name);
            let launch_fragment = scoot_look::render_launch(&look)
                .unwrap_or_else(|e| panic!("{name}: launch.toml does not render: {e}"));
            let fuzzel = scoot_look::render_fuzzel(&look)
                .unwrap_or_else(|e| panic!("{name}: fuzzel.ini does not render: {e}"));
            let launch = launch_colors(&launch_fragment);
            let fuzzel = fuzzel_colors(&fuzzel);
            let row = |launch_key: &str, fuzzel_key: &str| {
                assert_eq!(
                    rgb(&launch[launch_key]),
                    rgb(&fuzzel[fuzzel_key]),
                    "{name}: launch `{launch_key}` draws fuzzel `{fuzzel_key}`"
                );
            };
            row("surface", "background");
            row("foreground", "text");
            row("foreground", "prompt");
            row("match", "match");
            row("selection", "selection");
            row("selection-text", "selection-text");
            row("focus", "border");
            // The panel's own read of the same fragment agrees with the
            // golden: what the test above pins is what the panel draws.
            let (read, notes) = crate::look::parse(&launch_fragment);
            assert!(notes.is_empty(), "{name}: {notes:?}");
            assert_eq!(read.surface, rgb(&launch["surface"]), "{name}");
            assert_eq!(read.theme.hover, rgb(&launch["match"]), "{name}");
            assert_eq!(read.theme.accent, rgb(&launch["selection"]), "{name}");
            assert_eq!(read.focus, rgb(&launch["focus"]), "{name}");
        }
    }
}
