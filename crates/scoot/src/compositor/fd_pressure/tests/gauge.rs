//! The cached table reading: what bounds an observation's cost per burst.
//!
//! Review of PR #241 found that on a raised table the uncached observation
//! (a readdir of `/proc/self/fd`, linear in open fds) run once per accepted
//! connection let a 4000-connection storm freeze the compositor for 35.6 s.
//! These pin the fix: accepts share one cached reading and count themselves
//! against it, and a reading is reused for 20 times what it cost.
//!
//! The burst tests pin the reading first ([`pin_reading`]), so they are
//! deterministic whatever the machine's timing, and run the real accept path:
//! connections to the real listening socket of a real `State`, drained by one
//! dispatch of its event loop on this thread (the gauge is per thread).

use std::io::Read;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use super::super::{
    MAX_READING_LIFETIME, MIN_READING_LIFETIME, RESERVE_FDS, Table, forget_reading, note_opened,
    observations, pin_reading, reading_lifetime, table,
};
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::Harness;

/// Long enough that a pinned reading cannot expire during a test.
const PINNED: Duration = Duration::from_secs(600);

#[test]
fn a_reading_is_reused_for_twenty_times_its_cost() {
    assert_eq!(reading_lifetime(Duration::ZERO), MIN_READING_LIFETIME);
    assert_eq!(
        reading_lifetime(Duration::from_micros(10)),
        MIN_READING_LIFETIME
    );
    assert_eq!(
        reading_lifetime(Duration::from_millis(7)),
        Duration::from_millis(140)
    );
    // A preempted or stalled readdir cannot stretch the reading past the
    // ceiling, however long it took.
    assert_eq!(
        reading_lifetime(Duration::from_millis(100)),
        MAX_READING_LIFETIME
    );
    assert_eq!(reading_lifetime(Duration::MAX), MAX_READING_LIFETIME);
}

#[test]
fn back_to_back_readings_observe_once() {
    forget_reading();
    let before = observations();
    let first = table();
    for _ in 0..1000 {
        let _ = table();
    }
    // At most one refresh can land inside the loop if the first reading's
    // 1 ms minimum expired mid-loop on a slow machine; never 1000.
    let made = observations() - before;
    assert!(
        (1..=3).contains(&made),
        "{made} observations for 1001 readings"
    );
    assert!(first.is_some(), "the observer works on this machine");
}

#[test]
fn opened_fds_count_against_the_cached_reading() {
    pin_reading(
        Some(Table {
            used: 1000,
            soft: 65536,
        }),
        PINNED,
    );
    note_opened(3);
    assert_eq!(table().map(|t| t.used), Some(1003));
    forget_reading();
}

#[test]
fn an_unknown_reading_stays_unknown() {
    pin_reading(None, PINNED);
    note_opened(1);
    assert_eq!(table(), None);
    forget_reading();
}

/// A reading is reused only for its lifetime: once that has passed, the
/// next read observes again, and the count it had accumulated is replaced by
/// the real one.
#[test]
fn an_expired_reading_is_taken_again() {
    pin_reading(
        Some(Table {
            used: 1_000_000,
            soft: 65536,
        }),
        Duration::from_millis(5),
    );
    assert_eq!(table().map(|t| t.used), Some(1_000_000), "still fresh");
    std::thread::sleep(Duration::from_millis(20));
    let before = observations();
    let fresh = table().expect("the observer works on this machine");
    assert_eq!(observations(), before + 1, "the expired reading was reused");
    assert!(
        fresh.used < 1_000_000,
        "the fresh reading is the real table: {fresh:?}"
    );
    forget_reading();
}

/// Connects `count` clients to `fixture`'s listening socket without
/// dispatching, so they are all in the backlog when the loop next runs.
fn connect_burst(fixture: &Fixture, count: usize) -> Vec<UnixStream> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").expect("a runtime dir");
    let path = std::path::Path::new(&runtime).join(&fixture.state.socket_name);
    (0..count)
        .map(|_| UnixStream::connect(&path).expect("a connection to the listening socket"))
        .collect()
}

type Fixture = Harness<(), ()>;

/// Every accept in a burst reads the cached figure, and the figure grows by
/// one per admitted connection: no observation at all, however many.
#[test]
fn an_accept_burst_uses_the_cached_reading() {
    let mut fixture: Fixture = Harness::bare(Appearance::default());
    fixture.settle();
    const BURST: usize = 300;
    let clients = connect_burst(&fixture, BURST);
    pin_reading(
        Some(Table {
            used: 100,
            soft: 65536,
        }),
        PINNED,
    );
    let before = observations();
    fixture
        .event_loop
        .dispatch(Some(Duration::ZERO), &mut fixture.state)
        .expect("a compositor dispatch");
    assert_eq!(
        observations(),
        before,
        "the accept burst observed the fd table instead of using the reading"
    );
    assert_eq!(
        table().map(|t| t.used),
        Some(100 + BURST as u64),
        "every admitted connection counted against the reading"
    );
    forget_reading();
    drop(clients);
}

/// The count is what judges the rest of the burst: with the reading ten fds
/// short of the pressure line, the first ten connections of a burst are
/// admitted and the rest are shed (EOF), all without a new observation.
#[test]
fn a_burst_that_crosses_the_line_sheds_its_tail() {
    let mut fixture: Fixture = Harness::bare(Appearance::default());
    fixture.settle();
    const BURST: usize = 50;
    const ROOM: u64 = 10;
    let soft = 65536;
    let clients = connect_burst(&fixture, BURST);
    pin_reading(
        Some(Table {
            used: soft - RESERVE_FDS - ROOM,
            soft,
        }),
        PINNED,
    );
    let before = observations();
    fixture
        .event_loop
        .dispatch(Some(Duration::ZERO), &mut fixture.state)
        .expect("a compositor dispatch");
    assert_eq!(observations(), before);
    forget_reading();
    let shed = clients
        .iter()
        .filter(|client| {
            client
                .set_read_timeout(Some(Duration::from_millis(200)))
                .expect("a read timeout");
            let mut byte = [0u8; 1];
            // A shed connection reads EOF; an admitted one has nothing to
            // read (the compositor sends nothing unprompted) and times out.
            matches!((&**client).read(&mut byte), Ok(0))
        })
        .count();
    assert_eq!(
        shed,
        BURST - (ROOM as usize + 1),
        "admitted up to the line (free == reserve still admits), shed the rest"
    );
}
