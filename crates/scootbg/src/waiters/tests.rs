use super::{Outcome, Progress, Waiters, outcome};

use Progress::{Done, Failed, Waiting};

#[test]
fn only_outputs_stamped_with_the_generation_or_later_count() {
    // Stamped 3: waiting. Generation 4 does not look at it.
    assert_eq!(outcome(4, [(3, Waiting)].into_iter()), Some(Outcome::Shown));
    assert_eq!(outcome(3, [(3, Waiting)].into_iter()), None);
    // A later request re-stamped it: generation 3 waits for that draw too,
    // which shows 3's change or one made after it.
    assert_eq!(outcome(3, [(5, Waiting)].into_iter()), None);
    assert_eq!(outcome(3, [(5, Done)].into_iter()), Some(Outcome::Shown));
}

#[test]
fn no_outputs_at_all_is_shown_at_once() {
    assert_eq!(outcome(1, std::iter::empty()), Some(Outcome::Shown));
}

#[test]
fn a_failure_is_reported_once_nothing_waits() {
    let list = [(2, Done), (2, Failed), (2, Waiting)];
    assert_eq!(outcome(2, list.into_iter()), None, "still waiting");
    let list = [(2, Done), (2, Failed), (1, Waiting)];
    assert_eq!(outcome(2, list.into_iter()), Some(Outcome::Failed));
}

#[test]
fn generations_start_at_one_and_increase() {
    let mut waiters = Waiters::<u32>::with_capacity(4);
    assert_eq!(waiters.next_generation(), 1);
    assert_eq!(waiters.next_generation(), 2);
}

/// Resolved in one turn: one sync for all of them; when it comes back,
/// their replies are ready, in order; the rest keep waiting.
#[test]
fn waiters_resolved_together_share_one_sync() {
    let mut waiters = Waiters::<u32>::with_capacity(8);
    waiters.push(10, 1);
    waiters.push(11, 2);
    waiters.push(12, 3);
    let sync = waiters.resolve(|generation| (generation != 2).then_some(Outcome::Shown));
    assert_eq!(sync, Some(1));
    assert_eq!(waiters.counts(), (1, 2));
    // Nothing more resolves: no sync.
    assert_eq!(waiters.resolve(|_| None), None);
    let mut ready = Vec::new();
    waiters.synced(1, |conn, outcome| ready.push((conn, outcome)));
    assert_eq!(ready, [(10, Outcome::Shown), (12, Outcome::Shown)]);
    // The last one fails, in a later turn, behind the next sync.
    assert_eq!(waiters.resolve(|_| Some(Outcome::Failed)), Some(2));
    ready.clear();
    waiters.synced(2, |conn, outcome| ready.push((conn, outcome)));
    assert_eq!(ready, [(11, Outcome::Failed)]);
    assert!(waiters.is_idle());
}

/// Syncs come back in order, so an answer for a later one releases the
/// earlier ones too; one for an earlier one leaves later ones in flight.
#[test]
fn a_sync_releases_every_earlier_one() {
    let mut waiters = Waiters::<u32>::with_capacity(8);
    waiters.push(1, 1);
    assert_eq!(waiters.resolve(|_| Some(Outcome::Shown)), Some(1));
    waiters.push(2, 2);
    assert_eq!(waiters.resolve(|_| Some(Outcome::Shown)), Some(2));
    let mut ready = Vec::new();
    waiters.synced(1, |conn, outcome| ready.push((conn, outcome)));
    assert_eq!(ready, [(1, Outcome::Shown)]);
    waiters.push(3, 3);
    assert_eq!(waiters.resolve(|_| Some(Outcome::Shown)), Some(3));
    ready.clear();
    waiters.synced(3, |conn, outcome| ready.push((conn, outcome)));
    assert_eq!(ready, [(2, Outcome::Shown), (3, Outcome::Shown)]);
    assert!(waiters.is_idle());
}

/// A connection that is gone takes its waiter with it, waiting or in
/// flight, and the others are untouched.
#[test]
fn waiters_of_gone_connections_are_forgotten() {
    let mut waiters = Waiters::<u32>::with_capacity(8);
    waiters.push(1, 1);
    waiters.push(2, 1);
    assert_eq!(waiters.resolve(|_| Some(Outcome::Shown)), Some(1));
    waiters.push(3, 2);
    waiters.push(4, 2);
    waiters.forget_gone(|conn| conn % 2 == 0);
    assert_eq!(waiters.counts(), (1, 1));
    let mut ready = Vec::new();
    waiters.synced(1, |conn, outcome| ready.push((conn, outcome)));
    assert_eq!(ready, [(2, Outcome::Shown)]);
    assert_eq!(waiters.resolve(|_| Some(Outcome::Shown)), Some(2));
    ready.clear();
    waiters.synced(2, |conn, outcome| ready.push((conn, outcome)));
    assert_eq!(ready, [(4, Outcome::Shown)]);
}

/// Within their capacity the lists never reallocate: the bound in the
/// module docs is what keeps them there.
#[test]
fn within_capacity_nothing_reallocates() {
    const CAPACITY: usize = 32;
    let mut waiters = Waiters::<usize>::with_capacity(CAPACITY);
    let (waiting, in_flight) = (waiters.waiting.as_ptr(), waiters.in_flight.as_ptr());
    let mut ready = Vec::with_capacity(CAPACITY);
    let ready_ptr = ready.as_ptr();
    for round in 0..10 {
        for conn in 0..CAPACITY {
            waiters.push(conn, round);
        }
        let sync = waiters.resolve(|_| Some(Outcome::Shown)).unwrap();
        ready.clear();
        waiters.synced(sync, |conn, outcome| ready.push((conn, outcome)));
        assert_eq!(ready.len(), CAPACITY);
    }
    assert_eq!(waiters.waiting.as_ptr(), waiting);
    assert_eq!(waiters.in_flight.as_ptr(), in_flight);
    assert_eq!(ready.as_ptr(), ready_ptr);
}
