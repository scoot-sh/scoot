//! What is owed to subscribers, decided without a compositor.

use super::*;

fn revisions(of: &[u64]) -> impl ExactSizeIterator<Item = u64> + '_ {
    of.iter().copied()
}

#[test]
fn nobody_subscribed_records_nothing() {
    let mut events = Events::default();
    assert!(!events.begin(false, revisions(&[1, 2])));
    events.note_output(true, Some("DP-1"));
    assert!(events.outputs.is_empty());
    assert!(events.told.is_empty());
}

#[test]
fn the_first_subscriber_arms_with_what_is_shown_then_as_the_baseline() {
    let mut events = Events::default();
    // The turn that sees the first subscriber only arms: nothing is owed
    // for what was already there.
    assert!(!events.begin(true, revisions(&[1, 2])));
    assert!(events.armed);
    assert!(events.begin(true, revisions(&[1, 2])));
    assert!(!events.pending(revisions(&[1, 2])));
    // A change is owed.
    assert!(events.pending(revisions(&[1, 3])));
}

#[test]
fn a_subscriber_joining_does_not_clear_what_is_owed_to_one_already_there() {
    let mut events = Events::default();
    events.begin(true, revisions(&[1, 2]));
    // Module 1 changes, and an output comes, and the batch is held back
    // by the frame gate (so it is still owed) ...
    events.note_output(true, Some("DP-2"));
    assert!(events.pending(revisions(&[1, 3])));
    // ... when a second subscriber joins. The daemon does nothing about it
    // but count it: the next turn sees `subscribed` as before.
    assert!(events.begin(true, revisions(&[1, 3])));
    assert!(
        events.pending(revisions(&[1, 3])),
        "the change was forgotten"
    );
    assert_eq!(events.outputs.len(), 1, "the output change was forgotten");
    assert_eq!(events.told, [1, 2]);
}

#[test]
fn the_last_subscriber_going_disarms_and_the_next_one_starts_fresh() {
    let mut events = Events::default();
    events.begin(true, revisions(&[1]));
    events.note_output(false, None);
    assert!(!events.begin(false, revisions(&[1])));
    assert!(!events.armed && events.outputs.is_empty() && events.told.is_empty());
    // A change made while nobody listened is not owed to the next one.
    assert!(!events.begin(true, revisions(&[5])));
    assert!(!events.pending(revisions(&[5])));
}

#[test]
fn a_reload_owes_every_module() {
    let mut events = Events::default();
    events.begin(true, revisions(&[1, 2]));
    events.invalidate();
    // Even with the same revisions (new modules start from their own).
    assert!(events.pending(revisions(&[1, 2])));
}

#[test]
fn output_changes_held_are_bounded_and_keep_the_newest() {
    let mut events = Events::default();
    events.begin(true, revisions(&[]));
    for n in 0..100 {
        events.note_output(n % 2 == 0, Some("DP-1"));
    }
    assert_eq!(events.outputs.len(), MAX_OUTPUT_CHANGES);
    // 99 is odd: the newest is a removal.
    assert!(!events.outputs.last().unwrap().added);
}
