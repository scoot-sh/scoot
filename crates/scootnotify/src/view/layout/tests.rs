//! The stack geometry's tests: card order, caps, wrap, hit areas.

use ab_glyph::{FontArc, FontVec};
use scootui::text::Text;

use super::*;
use crate::bus::hints::Urgency;
use crate::bus::notify::Action;
use crate::config::Config;
use crate::model::expiry::Clock;
use crate::model::store::{Icon, Notification};

fn text() -> Text {
    Text::new(FontArc::new(
        FontVec::try_from_vec(scootui::testfont::build()).unwrap(),
    ))
}

fn note(id: u32, app: &str, summary: &str, body: &str) -> Notification {
    Notification {
        id,
        app_name: app.to_owned(),
        sender: ":1.1".to_owned(),
        icon: Icon::None,
        summary: summary.to_owned(),
        body: body.to_owned(),
        plain_body: body.to_owned(),
        actions: Vec::new(),
        urgency: Urgency::Normal,
        category: String::new(),
        desktop_entry: String::new(),
        transient: false,
        resident: false,
        value: None,
        seq: u64::from(id),
        timeout_ms: Some(10_000),
        created_ms: 1_000,
        shown_ms: None,
        clock: Clock::waiting(),
    }
}

#[test]
fn newest_is_on_top_and_oldest_below() {
    let text = text();
    let config = Config::default();
    let first = note(1, "a", "first", "one");
    let second = note(2, "b", "second", "two");
    let stack = layout(&[&first, &second], &config, &text, 13.0, 2_000);
    assert_eq!(stack.cards.len(), 2);
    assert_eq!(stack.cards[0].id, 2);
    assert_eq!(stack.cards[1].id, 1);
    assert!(stack.cards[0].rect.y < stack.cards[1].rect.y);
    assert_eq!(stack.hidden, 0);
    assert!(stack.pill_label.is_empty());
}

#[test]
fn past_max_visible_a_pill_covers_the_rest() {
    let text = text();
    let config = Config {
        max_visible: 2,
        ..Config::default()
    };
    let notes: Vec<Notification> = (1..=4).map(|id| note(id, "a", "s", "b")).collect();
    let refs: Vec<&Notification> = notes.iter().collect();
    let stack = layout(&refs, &config, &text, 13.0, 2_000);
    assert_eq!(stack.cards.len(), 2);
    assert_eq!(stack.cards[0].id, 4);
    assert_eq!(stack.hidden, 2);
    assert_eq!(stack.pill_label, "+2 more");
    assert!(stack.height > stack.cards[1].rect.y + stack.cards[1].rect.height);
}

#[test]
fn the_input_region_is_the_cards_union() {
    let text = text();
    let config = Config::default();
    let first = note(1, "a", "first", "one");
    let second = note(2, "b", "second", "two");
    let stack = layout(&[&first, &second], &config, &text, 13.0, 2_000);
    assert_eq!(stack.input.len(), 2);
    assert_eq!(stack.input[0], stack.cards[0].rect);
    assert_eq!(stack.input[1], stack.cards[1].rect);
    // The gap between the cards is in neither rectangle.
    let gap_y = stack.cards[0].rect.y + stack.cards[0].rect.height;
    assert!(gap_y < stack.cards[1].rect.y);
}

#[test]
fn a_long_body_wraps_and_ellipsizes() {
    let text = text();
    let config = Config {
        body_lines: 2,
        ..Config::default()
    };
    let body = "word ".repeat(200);
    let single = note(1, "a", "s", &body);
    let stack = layout(&[&single], &config, &text, 13.0, 2_000);
    assert_eq!(stack.cards.len(), 1);
    assert_eq!(stack.cards[0].body.len(), 2);
    let last = stack.cards[0].body.last().unwrap();
    let tail: String = last.runs.iter().map(|run| run.text.as_str()).collect();
    assert!(tail.ends_with('…'), "{tail:?}");
}

#[test]
fn the_summary_caps_at_two_lines() {
    let text = text();
    let config = Config::default();
    let summary = "word ".repeat(200);
    let single = note(1, "a", &summary, "b");
    let stack = layout(&[&single], &config, &text, 13.0, 2_000);
    assert_eq!(stack.cards[0].summary.len(), 2);
}

#[test]
fn the_meta_freezes_the_age() {
    let text = text();
    let config = Config::default();
    let single = note(1, "mail", "s", "b");
    let early = layout(&[&single], &config, &text, 13.0, 2_000);
    let late = layout(&[&single], &config, &text, 13.0, 2_000_000);
    let meta = |stack: &Stack| {
        stack.cards[0]
            .meta
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>()
    };
    assert_eq!(meta(&early), "mail · now");
    assert_eq!(meta(&late), "mail · 33m");
}

#[test]
fn buttons_show_four_and_never_default() {
    let text = text();
    let config = Config::default();
    let mut single = note(1, "a", "s", "b");
    single.actions = (0..6)
        .map(|i| Action {
            key: if i == 0 {
                "default".to_owned()
            } else {
                format!("a{i}")
            },
            label: format!("Action {i}"),
        })
        .collect();
    let stack = layout(&[&single], &config, &text, 13.0, 2_000);
    assert_eq!(stack.cards[0].buttons.len(), 4);
    assert!(stack.cards[0].buttons.iter().all(|b| b.key != "default"));
    assert_eq!(stack.cards[0].buttons[0].key, "a1");
}

#[test]
fn an_empty_stack_has_no_size() {
    let text = text();
    let config = Config::default();
    let stack = layout(&[], &config, &text, 13.0, 2_000);
    assert_eq!((stack.width, stack.height), (360, 0));
    assert!(stack.cards.is_empty() && stack.input.is_empty());
}
