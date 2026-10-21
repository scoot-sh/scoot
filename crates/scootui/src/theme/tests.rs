use super::Theme;
use crate::class::Class;

#[test]
fn each_class_maps_to_its_token() {
    let theme = Theme::default();
    assert_eq!(theme.class(Class::Normal), theme.foreground);
    assert_eq!(theme.class(Class::Warn), theme.accent);
    assert_eq!(theme.class(Class::Urgent), theme.urgent);
    assert_eq!(theme.class(Class::Muted), theme.dim);
}

#[test]
fn hover_is_the_old_tint_until_set() {
    // The hover tint was the accent before it had a token of its own, so
    // the default keeps it: setting `[colors] hover` is what changes it.
    assert_eq!(Theme::default().hover, Theme::default().accent);
}

#[test]
fn the_classes_are_told_apart_by_default() {
    let theme = Theme::default();
    let mut colors = [
        theme.background,
        theme.class(Class::Normal),
        theme.class(Class::Warn),
        theme.class(Class::Urgent),
        theme.class(Class::Muted),
    ]
    .map(|c| c.xrgb8888());
    colors.sort_unstable();
    colors.windows(2).for_each(|w| assert_ne!(w[0], w[1]));
}

use super::SelectedMatch;
use crate::color::Color;

fn color(hex: &str) -> Color {
    Color::parse(hex).unwrap()
}

/// The shipped looks' panel roles (from `looks/*/look.toml`; radial-burst
/// sets no `hover`, so it defaults to `accent`), plus the built-in
/// default. `(background, accent, hover)` is all the rule reads.
const SHIPPED: &[(&str, &str, &str, &str)] = &[
    ("ginger-night", "#0E0E0E", "#E57F29", "#FFB14B"),
    ("moonrise", "#2B3648", "#FFA45C", "#FFD54A"),
    ("music-desk", "#FCFBFB", "#3D579A", "#5D7AB0"),
    ("radial-burst", "#241721", "#31a9e5", "#31a9e5"),
    ("vinyl-sunset", "#270E0A", "#FE9949", "#FEC69A"),
    ("built-in default", "#1e1e2e", "#f9e2af", "#f9e2af"),
];

fn theme_of(background: &str, accent: &str, hover: &str) -> Theme {
    Theme {
        background: color(background),
        foreground: color("#F5EAD6"),
        accent: color(accent),
        hover: color(hover),
        dim: color("#8F7A63"),
        urgent: color("#E05A4E"),
    }
}

/// The effective selected-row match ink's contrast against the selection
/// fill: `Ink` reads directly; the inverted chip reads as the chip pair
/// (whichever direction is the text on its ground).
fn effective_ratio(resolved: SelectedMatch, theme: &Theme) -> f64 {
    match resolved {
        SelectedMatch::Ink(ink) => ink.contrast_ratio(theme.accent),
        SelectedMatch::Inverted => theme.background.contrast_ratio(theme.accent),
    }
}

#[test]
fn match_on_selection_contrasts_in_every_shipped_look() {
    for (name, background, accent, hover) in SHIPPED {
        let theme = theme_of(background, accent, hover);
        let resolved = theme.match_on_selection();
        assert!(
            effective_ratio(resolved, &theme) >= 3.0,
            "{name}: {resolved:?} below 3:1"
        );
    }
}

#[test]
fn match_on_selection_is_the_inverted_chip_where_hover_fails() {
    // Every shipped look's `hover` sits on `accent` below 3:1 (1.0-1.6),
    // while the row's own text ink clears 6:1, so all resolve inverted.
    for (name, background, accent, hover) in SHIPPED {
        let theme = theme_of(background, accent, hover);
        assert_eq!(
            theme.match_on_selection(),
            SelectedMatch::Inverted,
            "{name}"
        );
    }
}

#[test]
fn match_on_selection_keeps_a_readable_match_color_on_both_rows() {
    // A `--match-color` that contrasts with the selection keeps meaning
    // what its name says: the selected row draws it too.
    let mut theme = theme_of("#0E0E0E", "#E57F29", "#FFB14B");
    theme.hover = color("#000000");
    assert_eq!(
        theme.match_on_selection(),
        SelectedMatch::Ink(color("#000000"))
    );
}

#[test]
fn match_on_selection_survives_a_hostile_look() {
    // Fill, text ink and hover all one mid gray: both measured pairs
    // fail, so the rule falls back to black or white ink, which always
    // clears 4.6:1 against any fill.
    let gray = Theme {
        background: color("#777777"),
        foreground: color("#777777"),
        accent: color("#777777"),
        hover: color("#777777"),
        dim: color("#777777"),
        urgent: color("#777777"),
    };
    let resolved = gray.match_on_selection();
    assert!(
        matches!(resolved, SelectedMatch::Ink(_)),
        "hostile look must not resolve inverted: {resolved:?}"
    );
    assert!(effective_ratio(resolved, &gray) >= 3.0);
}
