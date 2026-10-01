use serde_json::json;

use super::*;
use crate::modules::harness::Harness;
use crate::modules::{Class, MAX_TEXT};

fn started(placeholder: &str) -> Harness {
    Harness::new(start(&Settings {
        placeholder: placeholder.into(),
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
