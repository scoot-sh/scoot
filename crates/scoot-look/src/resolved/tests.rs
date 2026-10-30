//! Tests for resolution defaults: optional roles derive from required
//! ones, and the wallpaper fill defaults to the background.

use std::collections::BTreeMap;

use super::pt_to_px;
use crate::resolve::resolve;
use crate::schema::{Origin, parse_look_toml};

#[test]
fn defaults_derive_from_required_roles() {
    let text = crate::schema::tests::minimal_toml();
    let raw = parse_look_toml(text.as_bytes()).expect("parses");
    let map = BTreeMap::from([("moonrise".to_string(), (raw, Origin::User))]);
    let look = resolve(&map, "moonrise").expect("resolves");
    assert_eq!(look.palette.surface, look.palette.background);
    assert_eq!(look.palette.hover, look.palette.accent);
    assert_eq!(look.palette.focus, look.palette.accent);
    assert_eq!(look.palette.border, look.palette.dim);
    assert_eq!(look.palette.accent_fg, look.palette.background);
    assert_eq!(look.palette.selection, look.palette.accent);
    assert_eq!(look.palette.selection_text, look.palette.accent_fg);
    assert_eq!(look.palette.r#match, look.palette.hover);
    assert_eq!(look.fonts.popup_size, look.fonts.size);
    assert_eq!(look.fonts.app_size, 12.0);
    assert_eq!(look.fonts.terminal_size, look.fonts.size);
    assert_eq!(look.opacity.bar, 1.0);
    assert_eq!(look.shape.ring_inactive_width, look.shape.ring_width);
    assert_eq!(look.cursor.accent, "#FF9A49");
    assert_eq!(
        look.wallpaper.as_ref().and_then(|w| w.fill.clone()),
        Some(look.palette.background.clone())
    );
    assert_eq!(pt_to_px(look.fonts.size), 13);
}

#[test]
fn effects_inherit_and_a_child_overrides_per_key() {
    let base_text = crate::schema::tests::minimal_toml().replacen(
        "[cursor]",
        "[effects]\nshadow = \"soft\"\nfill = \"gradient\"\n[cursor]",
        1,
    );
    let base = parse_look_toml(base_text.as_bytes()).expect("base parses");
    let child_text =
        "schema = 1\nname = \"child\"\nextends = \"base\"\n[effects]\nshadow = \"none\"\n";
    let child = parse_look_toml(child_text.as_bytes()).expect("child parses");
    let map = BTreeMap::from([
        ("base".to_string(), (base, Origin::User)),
        ("child".to_string(), (child, Origin::User)),
    ]);
    let look = resolve(&map, "child").expect("resolves");
    // `none` is a value, not an absence: the child cancels the parent's
    // shadow and still inherits its fill.
    assert_eq!(look.effects.shadow.as_deref(), Some("none"));
    assert_eq!(look.effects.fill.as_deref(), Some("gradient"));
}
