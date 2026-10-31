//! The look layer's tests: defaults, the merge, and refusals.

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
    assert_eq!(look.border, Color::parse("#123456").unwrap());
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
    assert!(notes.iter().any(|note| note.contains("other")));
}

#[test]
fn hostile_input_never_panics() {
    for hostile in [
        "font-size = -3\nradius = -9\nopacity = 2.5\n",
        "font = \"a\0b\"\n[colors]\nbackground = \"#12345\"\n",
        "[colors]\nbackground = 3\nforeground = [\"#101010\"]\n",
        "[icons]\ntheme = \"../evil\"\ntheme = \".hidden\"\n",
        "opacity = \"opaque\"\nfont-size = 1.5\n",
        "[colors]\nborder = \"#1234567890abcdef\"\n",
    ] {
        let (look, notes) = parse(hostile);
        assert!(!notes.is_empty(), "{hostile:?}");
        let _ = look;
    }
    // A dangling symlink reads as an error, never a layer: covered by
    // `load` returning defaults with a note (no filesystem in this
    // module's tests beyond that contract).
    let (look, notes) = parse("[colors]\n");
    assert_eq!(look, Look::default());
    assert!(notes.is_empty());
}

#[test]
fn a_bom_is_not_hostile() {
    // TOML accepts the byte-order mark: the fragment parses, and the
    // key lands.
    let (look, notes) = parse("\u{feff}[colors]\nbackground = \"#101010\"\n");
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(look.theme.background, Color::parse("#101010").unwrap());
}

#[test]
fn the_low_border_is_its_own_role() {
    // `border` lands apart from `dim` and `focus`: the low-urgency ring
    // is neither the muted text nor the normal ring.
    let (look, notes) = parse("[colors]\nborder = \"#112233\"\n");
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(look.border, Color::parse("#112233").unwrap());
    assert_eq!(look.focus, Look::default().focus);
    assert_eq!(look.theme.dim, Look::default().theme.dim);
}

#[test]
fn the_depth_keys_land_and_default_to_off() {
    let defaults = Look::default();
    assert_eq!(defaults.shadow, ShadowStyle::None);
    assert_eq!(defaults.fill, FillStyle::Solid);
    assert!(defaults.shadow.spec().is_none(), "none builds nothing");
    for (name, style) in [
        ("none", ShadowStyle::None),
        ("soft", ShadowStyle::Soft),
        ("deep", ShadowStyle::Deep),
        ("glow", ShadowStyle::Glow),
    ] {
        let (look, notes) = parse(&format!("shadow = \"{name}\"\nfill = \"gradient\"\n"));
        assert!(notes.is_empty(), "{name}: {notes:?}");
        assert_eq!(look.shadow, style);
        assert_eq!(look.fill, FillStyle::Gradient);
        assert_eq!(look.shadow.spec().is_some(), style != ShadowStyle::None);
    }
}

#[test]
fn a_refused_depth_value_is_reported_and_ignored_never_fatal() {
    for text in [
        "shadow = \"huge\"",
        "shadow = 3",
        "shadow = \"Soft\"",
        "fill = \"rainbow\"",
        "fill = true",
    ] {
        let (look, notes) = parse(&format!("radius = 5\n{text}\n"));
        assert_eq!(look.shadow, ShadowStyle::None, "{text}");
        assert_eq!(look.fill, FillStyle::Solid, "{text}");
        // The rest of the fragment still lands.
        assert_eq!(look.radius, 5, "{text}");
        assert_eq!(notes.len(), 1, "{text}: {notes:?}");
        assert!(
            notes[0].contains("shadow") || notes[0].contains("fill"),
            "{notes:?}"
        );
    }
}

#[test]
fn the_shadow_styles_are_ordered_by_weight() {
    let spec = |style: ShadowStyle| style.spec().expect("a shadow");
    let (soft, deep, glow) = (
        spec(ShadowStyle::Soft),
        spec(ShadowStyle::Deep),
        spec(ShadowStyle::Glow),
    );
    assert!(soft.blur < deep.blur && soft.alpha < deep.alpha);
    assert_eq!((glow.offset_x, glow.offset_y), (0, 0), "a glow is centered");
    assert!(
        soft.offset_y > 0 && deep.offset_y > soft.offset_y,
        "shadows fall down"
    );
}

#[test]
fn a_gradient_puts_the_light_at_the_top_for_either_polarity() {
    let mut look = Look {
        surface: Color::parse("#101010").unwrap(),
        ..Look::default()
    };
    assert_eq!(
        look.fill_stops(),
        (look.surface, look.surface),
        "solid is flat"
    );
    look.fill = FillStyle::Gradient;
    let (top, bottom) = look.fill_stops();
    assert_eq!(bottom, look.surface);
    assert!(top.luminance() > bottom.luminance(), "dark: lightened top");
    look.surface = Color::parse("#f0f0f0").unwrap();
    let (top, bottom) = look.fill_stops();
    assert_eq!(top, look.surface);
    assert!(
        top.luminance() > bottom.luminance(),
        "light: darkened bottom"
    );
    // The text keeps its contrast across the card: the shift is small.
    let ink = Color::parse("#f5ead6").unwrap();
    look.surface = Color::parse("#0e0e0e").unwrap();
    let (top, bottom) = look.fill_stops();
    assert!(ink.contrast_ratio(top) > 4.5 && ink.contrast_ratio(bottom) > 4.5);
}

#[test]
fn a_fragment_without_motion_moves_nothing() {
    let (look, notes) = parse("radius = 8\n");
    assert!(notes.is_empty());
    assert_eq!(look.motion, MotionTokens::default());
    assert_eq!(look.motion.curve, Curve::None);
    assert_eq!(look.motion.timing(), Timing::NONE);
}

#[test]
fn every_motion_key_lands() {
    let (look, notes) = parse(
        "motion = \"spring\"\nmotion-duration-ms = 380\nmotion-bounce = 0.25\n\
         [card]\nenter = \"drop\"\nexit = \"scale\"\n",
    );
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(look.motion.curve, Curve::Spring);
    assert_eq!(look.motion.duration_ms, 380);
    assert_eq!(look.motion.bounce, 0.25);
    assert_eq!(look.motion.enter, CardMotion::Drop);
    assert_eq!(look.motion.exit, CardMotion::Scale);
    assert!(look.motion.timing().active());
}

#[test]
fn an_integer_bounce_is_a_bounce() {
    let (look, notes) = parse("motion-bounce = 1\n");
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(look.motion.bounce, 1.0);
}

#[test]
fn bad_motion_values_are_named_and_ignored() {
    let (look, notes) = parse(
        "motion = \"bouncy\"\nmotion-duration-ms = 99999\nmotion-bounce = 4.0\n\
         [card]\nenter = \"spin\"\nexit = 3\nleave = \"fade\"\n",
    );
    assert_eq!(look.motion, MotionTokens::default());
    for key in [
        "motion",
        "motion-duration-ms",
        "motion-bounce",
        "card.enter",
        "card.exit",
        "card.leave",
    ] {
        assert!(
            notes.iter().any(|note| note.contains(&format!("`{key}`"))),
            "{key}: {notes:?}"
        );
    }
    assert_eq!(notes.len(), 6, "{notes:?}");
}

#[test]
fn card_that_is_not_a_table_is_skipped() {
    let (look, notes) = parse("card = \"slide\"\n");
    assert_eq!(look.motion, MotionTokens::default());
    assert_eq!(notes.len(), 1, "{notes:?}");
}

#[test]
fn a_zero_duration_or_none_is_no_motion() {
    for text in [
        "motion = \"none\"\nmotion-duration-ms = 300\n",
        "motion = \"spring\"\nmotion-duration-ms = 0\n",
    ] {
        let (look, notes) = parse(text);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(look.motion.timing(), Timing::NONE, "{text}");
    }
}

#[test]
fn motion_tokens_clamp_even_when_built_by_hand() {
    let tokens = MotionTokens {
        curve: Curve::Spring,
        duration_ms: u32::MAX,
        bounce: f32::NAN,
        ..MotionTokens::default()
    };
    let timing = tokens.timing();
    assert_eq!(
        timing.duration(),
        std::time::Duration::from_millis(u64::from(scootui::motion::MAX_DURATION_MS))
    );
}

#[test]
fn a_fragment_without_card_styles_or_a_title_table_styles_nothing() {
    let (look, notes) = parse("radius = 8\n[card]\nenter = \"fade\"\n");
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(look.mark, CardMark::None);
    assert_eq!(look.shape, CardShape::Rounded);
    assert!(!look.app_accent);
    assert_eq!(look.title_weight, 700);
    assert_eq!(look.title_spacing, 0);
    assert_eq!(look.card_radius(Some(8)), 8);
    assert_eq!(look.title_strikes(), 2);
}

#[test]
fn every_card_style_and_title_key_lands() {
    let (look, notes) = parse(
        "[card]\nmark = \"badge\"\nshape = \"pill\"\napp-accent = true\n\
         [title]\nweight = 800\nspacing = 2\n",
    );
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(look.mark, CardMark::Badge);
    assert_eq!(look.shape, CardShape::Pill);
    assert!(look.app_accent);
    assert_eq!(look.title_weight, 800);
    assert_eq!(look.title_spacing, 2);
    // The pill ignores the configured radius; the paint cuts it to the card.
    assert_eq!(look.card_radius(Some(8)), style::PILL_RADIUS);
    assert_eq!(look.title_strikes(), 3);
    assert_eq!(look.title_tracking(), 2.0);
}

#[test]
fn bad_card_style_and_title_values_are_named_and_ignored() {
    for (text, key) in [
        ("[card]\nmark = \"ribbon\"\n", "card.mark"),
        ("[card]\nmark = 3\n", "card.mark"),
        ("[card]\nshape = \"round\"\n", "card.shape"),
        ("[card]\napp-accent = \"yes\"\n", "card.app-accent"),
        ("[card]\napp-accent = 1\n", "card.app-accent"),
        ("[title]\nweight = 50\n", "title.weight"),
        ("[title]\nweight = 901\n", "title.weight"),
        ("[title]\nweight = \"bold\"\n", "title.weight"),
        ("[title]\nspacing = 9\n", "title.spacing"),
        ("[title]\nspacing = -3\n", "title.spacing"),
        ("[title]\nspacing = 0.5\n", "title.spacing"),
        ("[title]\nfamily = \"x\"\n", "title.family"),
    ] {
        let (look, notes) = parse(text);
        assert_eq!(look, Look::default(), "{text}");
        assert!(
            notes.len() == 1 && notes[0].contains(key),
            "{text}: {notes:?}"
        );
    }
    let (look, notes) = parse("title = 3\n");
    assert_eq!(look, Look::default());
    assert!(notes[0].contains("not a table"), "{notes:?}");
}

#[test]
fn title_numbers_clamp_even_when_the_grammar_is_skipped() {
    let mut look = Look::default();
    let mut notes = Vec::new();
    for (value, weight, spacing) in [(i64::MAX, 900, 8), (i64::MIN, 100, -2)] {
        Slot::TitleWeight(&mut look.title_weight).set(
            &toml::Value::Integer(value),
            "title.weight",
            &mut notes,
        );
        Slot::TitleSpacing(&mut look.title_spacing).set(
            &toml::Value::Integer(value),
            "title.spacing",
            &mut notes,
        );
        assert_eq!((look.title_weight, look.title_spacing), (weight, spacing));
    }
}

#[test]
fn the_card_radius_is_the_files_over_the_looks() {
    let (look, _) = parse("radius = 14\n");
    assert_eq!(look.card_radius(None), 14, "the look's, with no file key");
    assert_eq!(look.card_radius(Some(3)), 3, "the file's key wins");
    assert_eq!(look.card_radius(Some(0)), 0, "a square file key is a key");
    assert_eq!(Look::default().card_radius(None), DEFAULT_RADIUS);
    let (pill, _) = parse("radius = 14\n[card]\nshape = \"pill\"\n");
    assert_eq!(pill.card_radius(None), style::PILL_RADIUS);
    assert_eq!(pill.card_radius(Some(3)), style::PILL_RADIUS);
}
