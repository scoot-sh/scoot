//! The bookkeeping on its own. What it means for a real connection -- a
//! refused client hearing why, a closed one giving its slot back, a
//! `wait-idle` waiter keeping hold of one -- is in `connection/tests.rs`,
//! against a real event loop.

use super::*;

#[test]
fn a_fresh_table_hands_out_every_slot_and_then_refuses() {
    let slots = Slots::new();
    let claimed: Vec<Slot> = (0..MAX_CONNECTIONS)
        .map(|nth| slots.claim().unwrap_or_else(|| panic!("slot {nth}")))
        .collect();
    assert_eq!(slots.live(), MAX_CONNECTIONS);
    assert!(
        slots.claim().is_none(),
        "a {}th connection was let in",
        MAX_CONNECTIONS + 1
    );
    // Still refusing, i.e. a refusal costs nothing and changes nothing: the
    // client in a loop this bound exists for asks again immediately.
    assert!(slots.claim().is_none());
    assert_eq!(slots.live(), MAX_CONNECTIONS);
    drop(claimed);
    assert_eq!(slots.live(), 0);
}

#[test]
fn a_dropped_slot_is_handed_out_again() {
    let slots = Slots::new();
    let first = slots.claim().expect("the first slot");
    assert_eq!(slots.live(), 1);
    drop(first);
    assert_eq!(slots.live(), 0);
    let _second = slots.claim().expect("the freed slot");
    assert_eq!(slots.live(), 1);
}

#[test]
fn slots_are_released_in_any_order() {
    // Nothing indexes a slot, so this is really a statement about the count:
    // releasing the oldest claim while newer ones are still out leaves room
    // for exactly one more, not for none and not for two.
    let slots = Slots::new();
    let first = slots.claim().expect("the first slot");
    let _second = slots.claim().expect("the second slot");
    drop(first);
    assert_eq!(slots.live(), 1);
    let _third = slots.claim().expect("the freed slot");
    assert_eq!(slots.live(), 2);
}
