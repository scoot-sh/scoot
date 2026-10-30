//! The app accent: stable, spread across hues, and always readable under
//! its ink, with the contrast math pinned against known WCAG pairs.

use super::*;

const WHITE: Color = Color {
    r: 255,
    g: 255,
    b: 255,
};
const CREAM: Color = Color {
    r: 0xf5,
    g: 0xea,
    b: 0xd6,
};

fn gray(level: u8) -> Color {
    Color {
        r: level,
        g: level,
        b: level,
    }
}

#[test]
fn the_contrast_math_matches_known_wcag_pairs() {
    // The classic AA line: white on #767676 passes at 4.54, #777777 is just
    // under at 4.48.
    let pass = WHITE.contrast_ratio(gray(0x76));
    let fail = WHITE.contrast_ratio(gray(0x77));
    assert!((pass - 4.54).abs() < 0.01, "{pass}");
    assert!((fail - 4.48).abs() < 0.01, "{fail}");
    assert!(pass >= ACCENT_CONTRAST && fail < ACCENT_CONTRAST);
    // And the extremes.
    assert!((WHITE.contrast_ratio(BLACK) - 21.0).abs() < 1e-9);
    assert!((WHITE.contrast_ratio(WHITE) - 1.0).abs() < 1e-9);
}

#[test]
fn an_accent_is_a_pure_stable_function_of_the_name() {
    let a = app_accent("firefox", CREAM).unwrap();
    assert_eq!(a, app_accent("firefox", CREAM).unwrap());
    // Pinned: a change here recolors every user's notifications, so it is
    // never an accident. FNV-1a, then splitmix64's finalizer, then hue.
    assert_eq!(name_hash("firefox") % 360, 308);
    assert_eq!(
        a,
        Color {
            r: 0xad,
            g: 0x32,
            b: 0x9d
        },
        "{a}"
    );
    // Case and surrounding space are not identity.
    assert_eq!(app_accent("Firefox", CREAM), app_accent("firefox", CREAM));
    assert_eq!(
        app_accent("  firefox ", CREAM),
        app_accent("firefox", CREAM)
    );
}

#[test]
fn a_name_with_nothing_in_it_has_no_accent() {
    for name in ["", " ", "\t\n"] {
        assert_eq!(app_accent(name, CREAM), None, "{name:?}");
    }
    // Anything else, however odd, gets one.
    for name in ["·", "日本語", "a\0b", &"x".repeat(10_000)] {
        assert!(app_accent(name, CREAM).is_some(), "{name:?}");
    }
}

#[test]
fn every_accent_reaches_the_contrast_under_every_ink() {
    let names: Vec<String> = (0..400)
        .map(|n| format!("app-{n}"))
        .chain(
            [
                "Firefox",
                "Slack",
                "Spotify",
                "org.gnome.Nautilus",
                "mail",
                "x",
            ]
            .map(String::from),
        )
        .collect();
    // Every gray, the shipped looks' foregrounds and the extremes.
    let inks: Vec<Color> = (0..=255u8)
        .step_by(3)
        .map(gray)
        .chain([WHITE, BLACK, CREAM, gray(0x77), gray(0x80), gray(0xbb)])
        .collect();
    for ink in &inks {
        for name in &names {
            let accent = app_accent(name, *ink).expect("a name has an accent");
            let ratio = ink.contrast_ratio(accent);
            assert!(
                ratio >= ACCENT_CONTRAST,
                "{name} under {ink}: {accent} is {ratio:.3}"
            );
        }
    }
}

#[test]
fn light_ink_darkens_and_dark_ink_lightens() {
    let base = |name: &str| {
        let hue = (name_hash(name) % 360) as f32;
        hsl(hue, 0.55, 0.50)
    };
    for name in ["firefox", "slack", "mail", "calendar"] {
        let b = base(name);
        let under_light = app_accent(name, WHITE).unwrap();
        let under_dark = app_accent(name, BLACK).unwrap();
        assert!(under_light.luminance() <= b.luminance(), "{name}");
        assert!(under_dark.luminance() >= b.luminance(), "{name}");
    }
}

#[test]
fn the_hues_spread() {
    let hues: std::collections::BTreeSet<u64> = (0..200)
        .map(|n| name_hash(&format!("app{n}")) % 360 / 30)
        .collect();
    assert_eq!(
        hues.len(),
        12,
        "200 similar names reach every 30-degree band"
    );
    let colors: std::collections::BTreeSet<(u8, u8, u8)> = (0..200)
        .map(|n| {
            let c = app_accent(&format!("app{n}"), CREAM).unwrap();
            (c.r, c.g, c.b)
        })
        .collect();
    assert!(
        colors.len() > 100,
        "{} distinct accents of 200",
        colors.len()
    );
}

#[test]
fn hsl_hits_the_primaries() {
    assert_eq!(hsl(0.0, 1.0, 0.5), Color { r: 255, g: 0, b: 0 });
    assert_eq!(hsl(120.0, 1.0, 0.5), Color { r: 0, g: 255, b: 0 });
    assert_eq!(hsl(240.0, 1.0, 0.5), Color { r: 0, g: 0, b: 255 });
    assert_eq!(hsl(0.0, 0.0, 0.5), gray(128));
    // The edge of the range does not index past the sectors.
    let _ = hsl(359.999, 0.55, 0.5);
    let _ = hsl(360.0, 0.55, 0.5);
}

#[test]
fn weights_step_through_one_two_and_three_strikes() {
    for (weight, strikes) in [
        (100, 1),
        (400, 1),
        (500, 1),
        (501, 2),
        (600, 2),
        (700, 2),
        (701, 3),
        (800, 3),
        (900, 3),
        (u16::MAX, 3),
        (0, 1),
    ] {
        assert_eq!(title_strikes(weight), strikes, "{weight}");
    }
    // The default weight is what a summary always drew: two strikes.
    assert_eq!(title_strikes(DEFAULT_TITLE_WEIGHT), 2);
}

#[test]
fn marks_and_shapes_parse_their_names_only() {
    for (name, mark) in [
        ("none", CardMark::None),
        ("stripe", CardMark::Stripe),
        ("dot", CardMark::Dot),
        ("badge", CardMark::Badge),
    ] {
        assert_eq!(CardMark::parse(name), Some(mark));
    }
    for name in ["", "Stripe", "bar", "pill"] {
        assert_eq!(CardMark::parse(name), None, "{name}");
    }
    assert_eq!(CardShape::parse("rounded"), Some(CardShape::Rounded));
    assert_eq!(CardShape::parse("pill"), Some(CardShape::Pill));
    assert_eq!(CardShape::parse("round"), None);
}
