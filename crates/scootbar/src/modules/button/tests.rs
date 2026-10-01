use super::*;
use crate::modules::harness::Harness;
use crate::modules::{MAX_TEXT, View};

fn view_of(settings: &Settings) -> View {
    Harness::new(start(settings)).view()
}

#[test]
fn a_button_shows_its_text_and_icon_and_never_wakes() {
    let settings = Settings::new("power", Some(Icon::Glyph('x')));
    let harness = Harness::new(start(&settings));
    assert_eq!(harness.source_count(), 0, "no fd, no wakeup");
    let view = harness.view();
    assert_eq!(view.text(), "power");
    assert_eq!(view.icon(), Some('x'));
}

#[test]
fn a_button_with_neither_shows_nothing() {
    assert!(view_of(&Settings::default()).is_empty());
    assert!(view_of(&Settings::new("   ", None)).is_empty());
}

#[test]
fn the_text_is_sanitized_and_bounded() {
    let view = view_of(&Settings::new(&format!("a\tb{}", "x".repeat(1000)), None));
    assert!(view.text().starts_with("a b"));
    assert_eq!(view.text().len(), MAX_TEXT);
    assert!(!view.was_cut());
    assert!(!view.text().chars().any(char::is_control));
}

#[test]
fn a_button_takes_no_set_value_and_no_pointer_of_its_own() {
    let mut module = start(&Settings::new("x", None));
    assert!(module.on_set(&serde_json::json!("y")).is_err());
    assert!(
        !module.handles_input(),
        "a binding, not the module, needs the pointer"
    );
}

#[test]
fn a_warm_view_allocates_nothing() {
    let module = start(&Settings::new("power", Some(Icon::Glyph('x'))));
    let mut view = View::default();
    module.view(&OutputView { name: None }, &mut view);
    let ((), allocations) = scootbg_mem::count_allocations(|| {
        for _ in 0..100 {
            view.clear();
            module.view(&OutputView { name: None }, &mut view);
        }
    });
    assert_eq!(allocations, 0);
}
