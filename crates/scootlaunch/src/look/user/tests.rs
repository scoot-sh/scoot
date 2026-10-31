//! The user layer's tests: precedence per key, bad values, the table
//! boundary, and the defaults staying byte-identical.

use std::path::PathBuf;

use super::*;
use crate::look::parse;
use scootui::effects::{FillStyle, ShadowStyle};
use scootui::motion::Curve;

fn path() -> PathBuf {
    PathBuf::from("/x/launch.toml")
}

/// The look a layer text resolves to, then the `[effects]` of `file`
/// over it, plus the notes the user layer added.
fn resolve(layer: &str, file: &str) -> (Look, Vec<String>) {
    let (mut look, layer_notes) = parse(layer);
    assert!(layer_notes.is_empty(), "{layer_notes:?}");
    let config = crate::config::parse(file.as_bytes()).expect("the file parses");
    let mut notes = Vec::new();
    apply_user(&mut look, &config, &path(), &mut notes);
    (look, notes)
}

const LAYER: &str = "shadow = \"soft\"\nfill = \"gradient\"\nmotion = \"ease-out\"\n\
    motion-duration-ms = 300\nmotion-bounce = 0.2\n";

#[test]
fn nothing_set_leaves_the_look_byte_identical() {
    let (layered, _) = parse(LAYER);
    let (look, notes) = resolve(LAYER, "");
    assert_eq!(look, layered);
    assert!(notes.is_empty());
    let (look, notes) = resolve("", "rows = 4\n");
    assert_eq!(look, Look::default());
    assert!(notes.is_empty());
}

#[test]
fn a_user_key_wins_and_every_other_key_stays_the_looks() {
    let (layered, _) = parse(LAYER);
    let (look, notes) = resolve(LAYER, "[effects]\nmotion = \"spring\"\n");
    assert!(notes.is_empty(), "{notes:?}");
    let mut expected = layered;
    expected.motion = Curve::Spring;
    assert_eq!(look, expected);
}

#[test]
fn each_user_key_wins_over_the_look_and_over_the_built_in() {
    let file = "[effects]\nshadow = \"glow\"\nfill = \"solid\"\nmotion = \"spring\"\n\
        motion-duration-ms = 120\nmotion-bounce = 0.5\n";
    for layer in [LAYER, ""] {
        let (look, notes) = resolve(layer, file);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(look.shadow, ShadowStyle::Glow);
        assert_eq!(look.fill, FillStyle::Solid);
        assert_eq!(look.motion, Curve::Spring);
        assert_eq!(look.motion_ms, 120);
        assert_eq!(look.motion_bounce, 0.5);
    }
}

#[test]
fn a_bad_value_is_a_note_and_the_look_keeps_its_value() {
    let (layered, _) = parse(LAYER);
    let (look, notes) = resolve(
        LAYER,
        "[effects]\nshadow = \"bogus\"\nfill = 1\nmotion-duration-ms = 5000\nmotion-bounce = 7.0\n",
    );
    assert_eq!(look, layered, "every refused key keeps the look's value");
    assert_eq!(notes.len(), 4, "{notes:?}");
    for note in &notes {
        assert!(note.starts_with("/x/launch.toml: "), "{note}");
    }
    assert!(notes.iter().any(|note| note.contains("`shadow` refused")));
}

#[test]
fn one_bad_key_does_not_stop_its_neighbours() {
    let (look, notes) = resolve("", "[effects]\nshadow = \"bogus\"\nmotion = \"spring\"\n");
    assert_eq!(notes.len(), 1);
    assert_eq!(look.shadow, ShadowStyle::None);
    assert_eq!(look.motion, Curve::Spring);
}

#[test]
fn the_table_reaches_only_the_effect_tokens() {
    for file in [
        "[effects]\nfont-size = 40\n",
        "[effects]\nopacity = 0.5\n",
        "[effects]\nradius = 4\n",
        "[effects]\n\"card.mark\" = \"dot\"\n",
        "[effects]\nenter = \"drop\"\n",
    ] {
        let error = crate::config::parse(file.as_bytes()).expect_err(file);
        assert!(error.contains("[effects] has no key"), "{file}: {error}");
    }
    // The notifier's card and title tables, the look's colors and icons,
    // and a table of the wrong shape are all refused.
    for file in [
        "[card]\nmark = \"dot\"\n",
        "[title]\nweight = 700\n",
        "[colors]\nsurface = \"#000000\"\n",
        "effects = 3\n",
    ] {
        assert!(crate::config::parse(file.as_bytes()).is_err(), "{file}");
    }
}

#[test]
fn a_hand_built_config_with_a_stray_key_is_noted_not_applied() {
    let mut config = crate::config::Config::default();
    config
        .effects
        .insert(String::from("font-size"), toml::Value::Integer(40));
    let mut look = Look::default();
    let mut notes = Vec::new();
    apply_user(&mut look, &config, &path(), &mut notes);
    assert_eq!(look, Look::default());
    assert_eq!(notes.len(), 1);
}
