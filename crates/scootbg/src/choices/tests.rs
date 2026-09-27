use super::Choices;
use crate::color::Color;

fn c(text: &str) -> Option<Color> {
    Some(Color::parse(text).unwrap())
}

#[test]
fn nothing_is_chosen_at_first() {
    let choices = Choices::default();
    assert_eq!(choices.for_output(Some("DP-1")), None);
    assert_eq!(choices.for_output(None), None);
}

#[test]
fn the_all_outputs_choice_covers_every_name_and_unnamed_outputs() {
    let mut choices = Choices::default();
    choices.set(None, c("#c03020"));
    assert_eq!(choices.for_output(Some("DP-1")), c("#c03020"));
    assert_eq!(choices.for_output(Some("HEADLESS-9")), c("#c03020"));
    assert_eq!(choices.for_output(None), c("#c03020"));
}

#[test]
fn a_named_choice_overrides_the_all_outputs_one_for_that_name_only() {
    let mut choices = Choices::default();
    choices.set(None, c("#c03020"));
    choices.set(Some("DP-2"), c("#101014"));
    assert_eq!(choices.for_output(Some("DP-2")), c("#101014"));
    assert_eq!(choices.for_output(Some("DP-1")), c("#c03020"));
    // Cleared by name: nothing there, the rest keep the color.
    choices.set(Some("DP-2"), None);
    assert_eq!(choices.for_output(Some("DP-2")), None);
    assert_eq!(choices.for_output(Some("DP-1")), c("#c03020"));
    assert_eq!(choices.named_len(), 1, "replaced in place, not appended");
}

#[test]
fn an_all_outputs_choice_replaces_every_named_one() {
    let mut choices = Choices::default();
    choices.set(Some("DP-1"), c("#111111"));
    choices.set(Some("DP-2"), None);
    choices.set(None, c("#222222"));
    assert_eq!(choices.named_len(), 0);
    assert_eq!(choices.for_output(Some("DP-1")), c("#222222"));
    assert_eq!(choices.for_output(Some("DP-2")), c("#222222"));
    // And clearing every output clears them all.
    choices.set(Some("DP-1"), c("#333333"));
    choices.set(None, None);
    assert_eq!(choices.for_output(Some("DP-1")), None);
}

#[test]
fn names_match_exactly() {
    let mut choices = Choices::default();
    choices.set(Some("DP-1"), c("#111111"));
    assert_eq!(choices.for_output(Some("dp-1")), None);
    assert_eq!(choices.for_output(Some("DP-10")), None);
    assert_eq!(choices.for_output(Some("DP-")), None);
}
