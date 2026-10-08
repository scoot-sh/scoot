//! [`Tracker`](super::Tracker): batches into per-output actives.

use super::Tracker;

/// One group carrying one output with two workspaces, the first active.
fn one_output_two(group: &mut Tracker, key: usize) {
    group.output_enter(key, "DP-1");
    let first = group.workspace_added(key).unwrap();
    group.workspace_named(key, first, "1");
    group.workspace_active(key, first, true);
    let second = group.workspace_added(key).unwrap();
    group.workspace_named(key, second, "2");
}

#[test]
fn the_batch_reports_once_at_done() {
    let mut tracker = Tracker::default();
    let key = tracker.group_added();
    // The events alone report nothing: only `done` closes the batch, so
    // no half-batch switch is ever reported.
    one_output_two(&mut tracker, key);
    assert_eq!(
        tracker.done(),
        vec![("DP-1".to_owned(), Some("1".to_owned()))]
    );
}

#[test]
fn nothing_reported_twice() {
    let mut tracker = Tracker::default();
    let key = tracker.group_added();
    one_output_two(&mut tracker, key);
    assert_eq!(
        tracker.done(),
        vec![("DP-1".to_owned(), Some("1".to_owned()))]
    );
    // No change: silence.
    assert!(tracker.done().is_empty());
}

#[test]
fn a_switch_reports_only_the_output_that_moved() {
    let mut tracker = Tracker::default();
    let left = tracker.group_added();
    tracker.output_enter(left, "DP-1");
    let l1 = tracker.workspace_added(left).unwrap();
    tracker.workspace_named(left, l1, "1");
    tracker.workspace_active(left, l1, true);
    let right = tracker.group_added();
    tracker.output_enter(right, "HDMI-A-1");
    let r1 = tracker.workspace_added(right).unwrap();
    tracker.workspace_named(right, r1, "1");
    tracker.workspace_active(right, r1, true);
    let r2 = tracker.workspace_added(right).unwrap();
    tracker.workspace_named(right, r2, "2");
    assert_eq!(
        tracker.done(),
        vec![
            ("DP-1".to_owned(), Some("1".to_owned())),
            ("HDMI-A-1".to_owned(), Some("1".to_owned())),
        ]
    );
    // Move only HDMI-A-1 to "2".
    tracker.workspace_active(right, r1, false);
    tracker.workspace_active(right, r2, true);
    assert_eq!(
        tracker.done(),
        vec![("HDMI-A-1".to_owned(), Some("2".to_owned()))]
    );
}

#[test]
fn a_group_with_two_outputs_is_ambiguous_and_silent() {
    let mut tracker = Tracker::default();
    let key = tracker.group_added();
    tracker.output_enter(key, "DP-1");
    tracker.output_enter(key, "HDMI-A-1");
    let first = tracker.workspace_added(key).unwrap();
    tracker.workspace_named(key, first, "1");
    tracker.workspace_active(key, first, true);
    // Neither output can be attributed: silence, not a guess.
    assert!(tracker.done().is_empty());
}

#[test]
fn a_group_with_no_output_is_silent() {
    let mut tracker = Tracker::default();
    let key = tracker.group_added();
    let first = tracker.workspace_added(key).unwrap();
    tracker.workspace_named(key, first, "1");
    tracker.workspace_active(key, first, true);
    assert!(tracker.done().is_empty());
}

#[test]
fn a_removed_group_reports_its_output_away_then_goes_quiet() {
    let mut tracker = Tracker::default();
    let key = tracker.group_added();
    one_output_two(&mut tracker, key);
    assert_eq!(
        tracker.done(),
        vec![("DP-1".to_owned(), Some("1".to_owned()))]
    );
    tracker.group_removed(key);
    assert_eq!(tracker.done(), vec![("DP-1".to_owned(), None)]);
    assert!(tracker.done().is_empty());
}

#[test]
fn a_removed_workspace_is_not_active() {
    let mut tracker = Tracker::default();
    let key = tracker.group_added();
    one_output_two(&mut tracker, key);
    assert_eq!(
        tracker.done(),
        vec![("DP-1".to_owned(), Some("1".to_owned()))]
    );
    // Workspace "1" removed while (wrongly) still flagged active: the
    // removal wins, and the output reports away rather than a dead name.
    tracker.workspace_removed(key, 0);
    assert_eq!(tracker.done(), vec![("DP-1".to_owned(), None)]);
}

#[test]
fn an_unnamed_workspace_never_becomes_active() {
    let mut tracker = Tracker::default();
    let key = tracker.group_added();
    tracker.output_enter(key, "DP-1");
    let unnamed = tracker.workspace_added(key).unwrap();
    tracker.workspace_active(key, unnamed, true);
    // No name announced: nothing attributable.
    assert_eq!(tracker.done(), vec![("DP-1".to_owned(), None)]);
    tracker.workspace_named(key, unnamed, "9");
    assert_eq!(
        tracker.done(),
        vec![("DP-1".to_owned(), Some("9".to_owned()))]
    );
}

#[test]
fn an_output_leaving_reports_away() {
    let mut tracker = Tracker::default();
    let key = tracker.group_added();
    one_output_two(&mut tracker, key);
    assert_eq!(
        tracker.done(),
        vec![("DP-1".to_owned(), Some("1".to_owned()))]
    );
    tracker.output_leave(key, "DP-1");
    assert_eq!(tracker.done(), vec![("DP-1".to_owned(), None)]);
}

#[test]
fn unknown_keys_are_ignored() {
    let mut tracker = Tracker::default();
    // No groups at all: every event is a no-op, and `done` is silent.
    tracker.output_enter(7, "DP-1");
    tracker.workspace_named(7, 0, "1");
    tracker.workspace_active(7, 0, true);
    tracker.workspace_removed(7, 0);
    tracker.group_removed(7);
    assert!(tracker.done().is_empty());
    assert_eq!(tracker.workspace_added(7), None);
}
