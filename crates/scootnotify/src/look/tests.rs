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
        "\u{feff}[colors]\nbackground = \"#101010\"\n",
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
fn the_low_border_is_its_own_role() {
    // `border` lands apart from `dim` and `focus`: the low-urgency ring
    // is neither the muted text nor the normal ring.
    let (look, notes) = parse("[colors]\nborder = \"#112233\"\n");
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(look.border, Color::parse("#112233").unwrap());
    assert_eq!(look.focus, Look::default().focus);
    assert_eq!(look.theme.dim, Look::default().theme.dim);
}
