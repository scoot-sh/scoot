//! The looks module on its own: the bar text follows the recorded choice,
//! the picker pages past the popup's widget cap with the active look
//! selected, a row stages `scoot look apply` for that snapshot's name, and
//! rows past the snapshot are refused rather than applied by position.

use std::path::PathBuf;

#[cfg(feature = "popup")]
use super::PAGE_ROWS;
use super::started;
#[cfg(feature = "popup")]
use crate::action::Action;
use crate::action::ModuleAction;
#[cfg(feature = "popup")]
use crate::icon::Icon;
#[cfg(feature = "popup")]
use crate::modules::Update;
use crate::modules::harness::Harness;
use crate::modules::{InvokeError, OutputView};
#[cfg(feature = "popup")]
use crate::popup::{Content, Kind};

const DP1: OutputView<'static> = OutputView { name: Some("DP-1") };

/// A scratch state dir, removed on drop.
fn scratch(tag: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("scootbar-looks-{tag}-{}-", std::process::id()))
        .tempdir()
        .expect("a scratch dir")
}

/// Records `name` as the active look in `dir`.
fn record(dir: &std::path::Path, name: &str) {
    let rendered = dir.join("rendered").join(name);
    std::fs::create_dir_all(&rendered).expect("a rendered dir");
    std::fs::write(
        dir.join("look.json"),
        format!(
            "{{\"name\":\"{name}\",\"rendered\":\"{}\",\"generation\":1}}",
            rendered.display()
        ),
    )
    .expect("a record");
}

fn up(dir: Option<PathBuf>) -> Harness {
    Harness::new(Box::new(started(None, dir)))
}

fn action(name: &'static str, arg: Option<i32>) -> ModuleAction {
    ModuleAction::new(name, arg)
}

/// Fills `looks` with `names` and `active`, returning the popup content.
#[cfg(feature = "popup")]
fn filled(looks: &mut super::Looks, names: &[String], active: Option<&str>) -> (bool, Content) {
    let mut content = Content::default();
    let has = looks.fill(names, active, &mut content);
    (has, content)
}

#[cfg(feature = "popup")]
fn labels(content: &Content) -> Vec<(String, &'static str, Option<i32>, bool, bool)> {
    content
        .widgets()
        .iter()
        .map(|widget| {
            let (action, arg, selected, closes) = match widget.kind {
                Kind::Button {
                    action,
                    arg,
                    selected,
                    closes,
                } => (action, arg, selected, closes),
                Kind::Text | Kind::Slider { .. } => ("", None, false, false),
            };
            (
                content.label(widget).to_owned(),
                action,
                arg,
                selected,
                closes,
            )
        })
        .collect()
}

fn names(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("look-{i:02}")).collect()
}

#[test]
fn the_bar_text_is_the_recorded_choice() {
    let scratch = scratch("recorded");
    record(scratch.path(), "moonrise");
    let harness = up(Some(scratch.path().to_owned()));
    assert_eq!(harness.view().text(), "moonrise");
    assert_eq!(harness.view_on(Some("DP-1")).text(), "moonrise");
}

#[test]
fn the_bar_text_stands_no_look_with_no_record() {
    let scratch = scratch("empty");
    let harness = up(Some(scratch.path().to_owned()));
    assert_eq!(harness.view().text(), "no look");
}

#[test]
fn the_bar_text_stands_no_look_on_a_corrupt_record() {
    let scratch = scratch("corrupt");
    std::fs::write(scratch.path().join("look.json"), "{nope").expect("a corrupt record");
    let harness = up(Some(scratch.path().to_owned()));
    assert_eq!(harness.view().text(), "no look");
}

#[test]
fn the_bar_text_stands_no_look_with_no_state_dir() {
    assert_eq!(up(None).view().text(), "no look");
}

#[test]
#[cfg(feature = "popup")]
fn a_short_catalog_is_one_page_with_the_active_selected() {
    let mut looks = started(None, None);
    let all = names(5);
    let (has, content) = filled(&mut looks, &all, Some("look-02"));
    assert!(has);
    assert_eq!(
        labels(&content),
        vec![
            ("look-00".to_owned(), "apply", Some(0), false, true),
            ("look-01".to_owned(), "apply", Some(1), false, true),
            ("look-02".to_owned(), "apply", Some(2), true, true),
            ("look-03".to_owned(), "apply", Some(3), false, true),
            ("look-04".to_owned(), "apply", Some(4), false, true),
        ]
    );
}

#[test]
#[cfg(feature = "popup")]
fn a_glyph_icon_marks_the_active_row() {
    let mut looks = started(Some(Icon::Glyph('●')), None);
    let all = names(3);
    let (has, content) = filled(&mut looks, &all, Some("look-01"));
    assert!(has);
    assert_eq!(
        labels(&content),
        vec![
            ("look-00".to_owned(), "apply", Some(0), false, true),
            ("● look-01".to_owned(), "apply", Some(1), true, true),
            ("look-02".to_owned(), "apply", Some(2), false, true),
        ]
    );
}

#[test]
#[cfg(feature = "popup")]
fn an_empty_catalog_is_no_popup() {
    let mut looks = started(None, None);
    let (has, _) = filled(&mut looks, &[], None);
    assert!(!has);
}

#[test]
#[cfg(feature = "popup")]
fn a_long_catalog_pages_past_the_widget_cap() {
    let mut looks = started(None, None);
    let all = names(30);
    let (has, content) = filled(&mut looks, &all, Some("look-29"));
    assert!(has);
    // 14 rows plus the next row: inside the 16-widget cap.
    assert_eq!(content.widgets().len(), PAGE_ROWS + 1);
    let rows = labels(&content);
    assert_eq!(rows[0].0, "look-00");
    assert!(!rows[0].3, "the first page selects nothing here");
    assert_eq!(
        rows[PAGE_ROWS],
        ("next ›".to_owned(), "page", Some(1), false, false)
    );
    // To the last page: 2 rows with a prev row, the active selected.
    // (Through `do_page` directly: the harness's popup would re-list the
    // live catalog, not the fixture.)
    assert_eq!(looks.do_page(2), Ok(Update::Changed));
    let (has, content) = filled(&mut looks, &all, Some("look-29"));
    assert!(has);
    let rows = labels(&content);
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0],
        ("‹ prev".to_owned(), "page", Some(1), false, false)
    );
    assert_eq!(rows[1].0, "look-28");
    assert_eq!(
        rows[2],
        ("look-29".to_owned(), "apply", Some(1), true, true)
    );
}

#[test]
#[cfg(feature = "popup")]
fn a_page_past_the_end_clamps_to_the_last() {
    let mut looks = started(None, None);
    looks.page = 9;
    let all = names(3);
    let (has, content) = filled(&mut looks, &all, None);
    assert!(has);
    assert_eq!(labels(&content).len(), 3);
    assert_eq!(looks.page, 0);
}

#[test]
#[cfg(feature = "popup")]
fn a_row_stages_scoot_look_apply_for_the_shown_name() {
    let mut looks = started(None, None);
    let all = names(3);
    let (has, _) = filled(&mut looks, &all, None);
    assert!(has);
    let mut harness = Harness::new(Box::new(looks));
    assert_eq!(
        harness.invoke(&DP1, &action("apply", Some(1)), 1),
        Ok(Update::Unchanged)
    );
    assert_eq!(
        harness.take_action(),
        Some(Action::Exec(vec![
            "scoot".to_owned(),
            "look".to_owned(),
            "apply".to_owned(),
            "look-01".to_owned(),
        ]))
    );
    // One shot: nothing staged twice.
    assert_eq!(harness.take_action(), None);
}

#[test]
#[cfg(feature = "popup")]
fn rows_past_the_snapshot_are_refused() {
    let mut looks = started(None, None);
    let all = names(2);
    let (has, _) = filled(&mut looks, &all, None);
    assert!(has);
    let mut harness = Harness::new(Box::new(looks));
    assert_eq!(
        harness.invoke(&DP1, &action("apply", Some(2)), 1),
        Err(InvokeError::Refused("no such look row"))
    );
    assert_eq!(
        harness.invoke(&DP1, &action("apply", Some(-1)), 1),
        Err(InvokeError::Refused("no such look row"))
    );
    assert_eq!(harness.take_action(), None);
}

#[test]
fn pages_past_the_end_are_refused() {
    let mut harness = up(None);
    assert_eq!(
        harness.invoke(&DP1, &action("page", Some(0)), 1),
        Err(InvokeError::Refused("no such page"))
    );
    assert_eq!(
        harness.invoke(&DP1, &action("page", Some(-1)), 1),
        Err(InvokeError::Refused("no such page"))
    );
}

#[test]
fn actions_without_a_number_need_one() {
    let mut harness = up(None);
    assert_eq!(
        harness.invoke(&DP1, &action("apply", None), 1),
        Err(InvokeError::NeedsArg)
    );
    assert_eq!(
        harness.invoke(&DP1, &action("page", None), 1),
        Err(InvokeError::NeedsArg)
    );
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("no-such-action", None), 1),
        Err(InvokeError::Unknown)
    );
}

#[test]
fn the_query_reports_the_active_look_and_the_page() {
    let scratch = scratch("query");
    record(scratch.path(), "moonrise");
    let harness = up(Some(scratch.path().to_owned()));
    let value = harness.value_on(None).expect("a value");
    assert_eq!(
        value["active"],
        serde_json::Value::String("moonrise".to_owned())
    );
    assert_eq!(value["page"], serde_json::Value::from(0));
}
