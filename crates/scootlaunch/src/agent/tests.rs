//! `--run-id` resolution pins: exact wins, suffix-insensitive is
//! unique-or-nothing, hostile ids never launch.

use super::*;
use crate::index::filter::HiddenReason;
use std::path::PathBuf;

fn entry(id: &str) -> Entry {
    Entry {
        id: id.to_string(),
        name: id.to_string(),
        generic_name: None,
        comment: None,
        keywords: Vec::new(),
        icon: None,
        exec: vec!["/usr/bin/app".to_string()],
        terminal: false,
        dbus_activatable: false,
        actions: Vec::new(),
        path: PathBuf::from("/x"),
        workdir: None,
        hidden: None,
    }
}

fn entry_with_actions(id: &str, actions: &[&str]) -> Entry {
    let mut base = entry(id);
    base.actions = actions
        .iter()
        .map(|name| Action {
            id: name.to_string(),
            name: name.to_string(),
            exec: vec!["/usr/bin/app".to_string()],
        })
        .collect();
    base
}

#[test]
fn exact_ids_resolve() {
    let entries = vec![entry("firefox.desktop"), entry("files.desktop")];
    let (found, action) = resolve(&entries, "firefox.desktop", None).unwrap();
    assert_eq!(found.id, "firefox.desktop");
    assert!(action.is_none());
}

#[test]
fn suffix_insensitive_resolves_when_unique() {
    let entries = vec![entry("firefox.desktop"), entry("files.desktop")];
    let (found, _) = resolve(&entries, "firefox", None).unwrap();
    assert_eq!(found.id, "firefox.desktop");
}

#[test]
fn suffix_insensitive_ambiguity_launches_nothing() {
    // Two entries whose stripped app ids collide: `a.desktop` and a
    // literal `a` id (a run-shaped id in the same index).
    let entries = vec![entry("a.desktop"), entry("a")];
    let error = resolve(&entries, "a", None).unwrap_err();
    assert_eq!(error.code(), "ambiguous-id");
}

#[test]
fn unknown_ids_do_not_launch() {
    let entries = vec![entry("firefox.desktop")];
    let error = resolve(&entries, "nothing.desktop", None).unwrap_err();
    assert_eq!(error.code(), "unknown-id");
    // A prefix never launches either.
    let error = resolve(&entries, "fire", None).unwrap_err();
    assert_eq!(error.code(), "unknown-id");
}

#[test]
fn hostile_ids_are_invalid_not_launches() {
    let entries = vec![entry("firefox.desktop")];
    assert_eq!(
        resolve(&entries, "", None).unwrap_err().code(),
        "invalid-id"
    );
    assert_eq!(
        resolve(&entries, &"A".repeat(MAX_ID_LEN + 1), None)
            .unwrap_err()
            .code(),
        "invalid-id"
    );
    assert_eq!(
        resolve(&entries, "fire\nfox.desktop", None)
            .unwrap_err()
            .code(),
        "invalid-id"
    );
    assert_eq!(
        resolve(&entries, "fire\x00fox.desktop", None)
            .unwrap_err()
            .code(),
        "invalid-id"
    );
    // The message never carries the raw newline.
    let error = resolve(&entries, "fire\nfox.desktop", None).unwrap_err();
    assert!(!error.message().contains('\n'), "{}", error.message());
}

#[test]
fn hidden_entries_still_resolve() {
    let mut hidden = entry("secret.desktop");
    hidden.hidden = Some(HiddenReason::NoDisplay);
    let entries = vec![hidden];
    let (found, _) = resolve(&entries, "secret.desktop", None).unwrap();
    assert_eq!(found.id, "secret.desktop");
}

#[test]
fn actions_resolve_exactly() {
    let entries = vec![entry_with_actions(
        "firefox.desktop",
        &["new-window", "new-private-window"],
    )];
    let (_, action) = resolve(&entries, "firefox.desktop", Some("new-window")).unwrap();
    assert_eq!(action.map(|action| action.id.as_str()), Some("new-window"));
    let error = resolve(&entries, "firefox.desktop", Some("nope")).unwrap_err();
    assert_eq!(error.code(), "unknown-action");
    let error = resolve(&entries, "firefox.desktop", Some("")).unwrap_err();
    assert_eq!(error.code(), "invalid-action");
    let error = resolve(&entries, "firefox.desktop", Some("a\nb")).unwrap_err();
    assert_eq!(error.code(), "invalid-action");
}

#[test]
fn success_and_error_objects_round_trip() {
    let ok = RunOk {
        ok: true,
        pid: Some(4242),
        scope: Some("app-scootlaunch-x-1.scope".to_string()),
        activated: None,
        detached: None,
    };
    let json = serde_json::to_string(&ok).unwrap();
    assert!(json.contains("\"pid\":4242"), "{json}");
    assert!(json.contains("app-scootlaunch"), "{json}");
    let back: RunOk = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ok);
    // Absent optionals serialize away rather than as nulls.
    let minimal = RunOk {
        ok: true,
        pid: None,
        scope: None,
        activated: Some(true),
        detached: None,
    };
    let json = serde_json::to_string(&minimal).unwrap();
    assert!(!json.contains("null"), "{json}");
    assert!(json.contains("activated"), "{json}");
    let error = RunErr::launch_failed("cannot start x: nope");
    let json = serde_json::to_string(&error).unwrap();
    assert!(json.contains("launch-failed"), "{json}");
    let back: RunErr = serde_json::from_str(&json).unwrap();
    assert_eq!(back, error);
}

#[test]
fn launch_failures_sanitize_control_characters() {
    let error = RunErr::launch_failed("bad\nline\x00here");
    assert!(!error.error.contains('\n'), "{}", error.error);
    assert!(!error.error.contains('\x00'), "{}", error.error);
}
