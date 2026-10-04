use serde_json::json;

use super::*;
use crate::modules::harness::Harness;
use crate::modules::{Class, MAX_TEXT};

fn started(placeholder: &str) -> Harness {
    Harness::new(start(&Settings {
        placeholder: placeholder.into(),
        icon: None,
        show_text: true,
    }))
}

fn set(harness: &mut Harness, value: Value) -> Result<Update, SetError> {
    harness.set(&value)
}

#[test]
fn it_shows_the_placeholder_until_the_first_set() {
    let mut harness = started("...");
    assert_eq!(harness.source_count(), 0, "no fd of its own");
    assert_eq!(harness.view().text(), "...");
    assert_eq!(set(&mut harness, json!("sunny")), Ok(Update::Changed));
    assert_eq!(harness.view().text(), "sunny");
}

#[test]
fn with_no_placeholder_it_shows_nothing_and_takes_no_space() {
    let mut harness = started("");
    assert!(harness.view().is_empty());
    let _ = set(&mut harness, json!("x")).unwrap();
    assert!(!harness.view().is_empty());
    // Cleared again.
    assert_eq!(set(&mut harness, json!(null)), Ok(Update::Changed));
    assert!(harness.view().is_empty());
}

#[test]
fn an_object_sets_text_class_and_tooltip() {
    let mut harness = started("");
    let _ = set(
        &mut harness,
        json!({"text": "72%", "class": "warn", "tooltip": "low"}),
    )
    .unwrap();
    let view = harness.view();
    assert_eq!(
        (view.text(), view.class(), view.tooltip()),
        ("72%", Class::Warn, "low")
    );
}

#[test]
fn a_set_that_changes_nothing_is_not_a_redraw() {
    let mut harness = started("");
    assert_eq!(set(&mut harness, json!("a")), Ok(Update::Changed));
    assert_eq!(set(&mut harness, json!("a")), Ok(Update::Unchanged));
    assert_eq!(
        set(&mut harness, json!({"text": "a", "class": "normal"})),
        Ok(Update::Unchanged)
    );
    assert_eq!(
        set(&mut harness, json!({"text": "a", "class": "warn"})),
        Ok(Update::Changed)
    );
}

#[test]
fn a_refused_value_leaves_what_was_shown() {
    let mut harness = started("keep");
    for bad in [
        json!(5),
        json!([1]),
        json!({"text": 1}),
        json!({"class": "loud"}),
        json!({"version": 2}),
        json!("x".repeat(100_000)),
    ] {
        let error = set(&mut harness, bad).unwrap_err();
        assert!(matches!(error, SetError::Invalid(_)), "{error}");
        assert_eq!(harness.view().text(), "keep");
    }
}

#[test]
fn the_view_is_bounded_whatever_is_set() {
    let mut harness = started("");
    let _ = set(
        &mut harness,
        json!({"text": "é".repeat(4000), "tooltip": "t".repeat(4000)}),
    )
    .unwrap();
    let view = harness.view();
    assert!(view.text().len() <= MAX_TEXT && view.tooltip().len() <= MAX_TEXT);
    assert!(!view.was_cut());
    assert!(!view.text().chars().any(char::is_control));
}

#[test]
fn a_flood_of_sets_ends_in_the_last() {
    let mut harness = started("");
    for i in 0..200 {
        let _ = set(&mut harness, json!(format!("n{}", i % 10))).unwrap();
    }
    assert_eq!(harness.view().text(), "n9");
}

#[test]
fn a_static_icon_is_drawn_and_an_updates_glyph_wins() {
    use crate::icon::Icon;
    let mut harness = Harness::new(start(&Settings {
        placeholder: "".into(),
        icon: Some(Icon::Glyph('s')),
        show_text: true,
    }));
    assert_eq!(harness.view().icon(), Some('s'));
    let _ = set(&mut harness, json!({"text": "hot", "icon": "u"})).unwrap();
    let view = harness.view();
    assert_eq!((view.text(), view.icon()), ("hot", Some('u')));
    // A line without one falls back to the static icon.
    let _ = set(&mut harness, json!({"text": "warm"})).unwrap();
    let view = harness.view();
    assert_eq!((view.text(), view.icon()), ("warm", Some('s')));
    // A refused icon changes nothing, static icon included.
    let before = harness.view().icon();
    assert!(set(&mut harness, json!({"icon": "two"})).is_err());
    assert_eq!(harness.view().icon(), before);
    // The icon reaches `query` as the module's glyph.
    assert_eq!(harness.view().icon(), Some('s'));
}

#[test]
fn show_text_false_draws_only_the_icon() {
    use crate::icon::Icon;
    let mut harness = Harness::new(start(&Settings {
        placeholder: "idle".into(),
        icon: Some(Icon::Glyph('s')),
        show_text: false,
    }));
    // The placeholder is text like any update, so it hides too, with its
    // text moved into the tooltip.
    assert_eq!(harness.view().text(), "");
    assert_eq!(harness.view().tooltip(), "idle");
    assert_eq!(harness.view().icon(), Some('s'));
    let _ = set(&mut harness, json!({"text": "hot", "tooltip": "CPU hot"})).unwrap();
    let view = harness.view();
    assert_eq!((view.text(), view.tooltip()), ("", "CPU hot"));
    assert_eq!(view.icon(), Some('s'));
}
