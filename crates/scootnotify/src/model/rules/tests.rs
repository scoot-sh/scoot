//! Rule order and merging: first match narrows, later keys win, every
//! `exec` accumulates.

use crate::bus::hints::Urgency;
use crate::config::{Rule, RuleUrgency};

use super::{Timeout, decide, matches};

fn ruled(rules: &[Rule]) -> super::Decision {
    decide(rules, "Mail", "mail.desktop", "email.arrived", Urgency::Normal, "New mail")
}

#[test]
fn no_rule_changes_nothing() {
    let decision = ruled(&[]);
    assert_eq!(decision, super::Decision::default());
}

#[test]
fn an_empty_rule_matches_everything() {
    assert!(matches(
        &Rule::default(),
        "anything",
        "anything.desktop",
        "anything",
        Urgency::Critical,
        "anything at all"
    ));
}

#[test]
fn every_match_key_narrows() {
    let rule = Rule {
        app_name: Some("Mail".to_owned()),
        desktop_entry: Some("mail.desktop".to_owned()),
        category: Some("email.arrived".to_owned()),
        urgency: Some(RuleUrgency::Normal),
        summary_contains: Some("New".to_owned()),
        ..Rule::default()
    };
    assert!(matches(
        &rule,
        "Mail",
        "mail.desktop",
        "email.arrived",
        Urgency::Normal,
        "New mail"
    ));
    assert!(!matches(
        &rule,
        "Chat",
        "mail.desktop",
        "email.arrived",
        Urgency::Normal,
        "New mail"
    ));
    assert!(!matches(
        &rule,
        "Mail",
        "chat.desktop",
        "email.arrived",
        Urgency::Normal,
        "New mail"
    ));
    assert!(!matches(
        &rule,
        "Mail",
        "mail.desktop",
        "chat.arrived",
        Urgency::Normal,
        "New mail"
    ));
    assert!(!matches(
        &rule,
        "Mail",
        "mail.desktop",
        "email.arrived",
        Urgency::Low,
        "New mail"
    ));
    assert!(!matches(
        &rule,
        "Mail",
        "mail.desktop",
        "email.arrived",
        Urgency::Normal,
        "Old mail"
    ));
}

#[test]
fn a_trailing_dot_is_a_class_prefix() {
    let rule = Rule {
        category: Some("device.".to_owned()),
        ..Rule::default()
    };
    assert!(matches(&rule, "a", "b", "device.added", Urgency::Normal, "s"));
    assert!(matches(&rule, "a", "b", "device", Urgency::Normal, "s"));
    assert!(!matches(&rule, "a", "b", "devicex", Urgency::Normal, "s"));
    assert!(!matches(&rule, "a", "b", "other.added", Urgency::Normal, "s"));
}

#[test]
fn later_scalars_win_and_execs_accumulate() {
    let first = Rule {
        app_name: Some("Mail".to_owned()),
        set_urgency: Some(RuleUrgency::Low),
        timeout_ms: Some(1000),
        output: Some("HDMI-1".to_owned()),
        exec: Some(vec!["a".to_owned()]),
        ..Rule::default()
    };
    let second = Rule {
        category: Some("email.".to_owned()),
        set_urgency: Some(RuleUrgency::Critical),
        timeout_ms: Some(0),
        dnd_bypass: true,
        hide: true,
        history: false,
        exec: Some(vec!["b".to_owned(), "c".to_owned()]),
        ..Rule::default()
    };
    let decision = ruled(&[first, second]);
    // Later keys win.
    assert_eq!(decision.urgency, Some(Urgency::Critical));
    assert_eq!(decision.timeout, Some(Timeout::Never));
    assert!(decision.hide);
    assert!(!decision.keep_history);
    assert_eq!(decision.output.as_deref(), None);
    assert!(decision.dnd_bypass);
    // Every matching exec runs, in rule order.
    assert_eq!(decision.execs.len(), 2);
    assert_eq!(decision.execs[0], vec!["a".to_owned()]);
    // A non-matching rule contributes nothing.
    let other = Rule {
        app_name: Some("Chat".to_owned()),
        set_urgency: Some(RuleUrgency::Low),
        exec: Some(vec!["z".to_owned()]),
        ..Rule::default()
    };
    let decision = ruled(&[other]);
    assert_eq!(decision, super::Decision::default());
}

/// Reverting the later-wins assignment to first-wins leaves the merge
/// test failing: the order pin is load-bearing, not a comment.
#[test]
fn history_false_is_sticky_but_output_is_later_wins() {
    let hide_history = Rule {
        history: false,
        output: Some("VGA-1".to_owned()),
        ..Rule::default()
    };
    let plain = Rule::default();
    let decision = ruled(&[hide_history, plain]);
    assert!(!decision.keep_history);
    assert_eq!(decision.output.as_deref(), Some("VGA-1"));
}
