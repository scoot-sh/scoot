//! The configure lookup's index (`popup_index.rs`): every tracked popup
//! is filed, every destroy forgets it, and a burst of clients each under
//! the per-client cap is served end to end -- the aggregate shape from
//! `docs/backlog/core/popup-aggregate-pressure-cap.md`.
//!
//! Every test drives a real `wayland-client` against a real headless
//! compositor. Each popup a test maps proves its first commit found it
//! through the index: `Step::Map` waits for that popup's configure serial,
//! which is only ever sent by `send_popup_initial_configure`. Nothing here
//! asserts wall time (a threshold is a flake); the speed is pinned by the
//! `scripts/popup-flood/run-many.sh` before/after numbers in the PR, and
//! these pin the exactness the speed relies on.

use super::*;

use crate::compositor::popup_count::MAX_POPUPS_PER_CLIENT;

/// The per-client cap, spelled out so this file does not depend on it
/// (see the module doc).
const COUNT_BOUND: usize = 128;

/// How many clients the aggregate test connects, each holding
/// [`COUNT_BOUND`] popups: 1024 live popups with nobody past its cap.
const AGGREGATE_CLIENTS: usize = 8;

fn siblings(count: usize) -> Vec<Op> {
    std::iter::repeat_n(Op::Popup(Parent::Window(0)), count).collect()
}

fn indexed(fixture: &Fixture) -> usize {
    fixture.state.popup_index.len()
}

/// Every tracked popup is filed, and destroying them forgets each one:
/// the index holds exactly the live popups, beside the per-client count.
#[test]
fn the_index_files_every_popup_and_drains_on_destroy() {
    let mut fixture = Fixture::with_window();
    fixture.batch(siblings(5));
    assert_eq!(indexed(&fixture), 5);
    // Each mapped popup proves its first commit read it back through the
    // index: `Map` waits for every popup's configure.
    fixture.map(0..5, Some(4));
    assert_eq!(indexed(&fixture), 5, "mapping keeps every entry");
    fixture.batch((0..5).map(Op::Destroy).collect());
    assert_eq!(indexed(&fixture), 0);
    assert_eq!(fixture.state.popup_count.in_flight(), 0);
}

/// Killing a client drains its entries with its teardown: the release
/// reads nothing back from the dead client, so the index must already
/// have forgotten each popup when its `xdg_popup` died.
#[test]
fn the_index_drains_when_a_client_is_killed() {
    assert_eq!(
        MAX_POPUPS_PER_CLIENT, COUNT_BOUND as u32,
        "the test's spelled-out cap drifted from the code's"
    );
    let mut fixture = Fixture::with_window();
    fixture.batch(siblings(COUNT_BOUND));
    assert_eq!(indexed(&fixture), COUNT_BOUND);
    let error = fixture.refused(siblings(1));
    assert!(error.contains("wl_display"), "{error}");
    assert_eq!(indexed(&fixture), 0, "the killed client's entries drained");
    assert_eq!(fixture.state.popup_count.in_flight(), 0);
}

/// Re-making a popup over its old surface refreshes the entry: the old
/// popup's destroy removes it before the new creation files afresh, so
/// the reincarnated popup is still configured through the index.
#[test]
fn reincarnating_a_popup_refreshes_the_index() {
    let mut fixture = Fixture::with_window();
    fixture.batch(siblings(COUNT_BOUND));
    fixture.batch(vec![Op::Reincarnate {
        popup: 0,
        parent: Parent::Window(0),
    }]);
    assert_eq!(
        indexed(&fixture),
        COUNT_BOUND,
        "the destroy and the re-creation cancelled out"
    );
    fixture.map(0..COUNT_BOUND, None);
    fixture.render();
    still_serving(&mut fixture);
}

/// The aggregate shape: several clients each at the per-client bound are
/// all admitted, every one of their popups configured, and one client's
/// destroys leave the others' entries alone.
#[test]
fn aggregate_clients_each_at_the_bound_are_all_served() {
    let mut fixture = Fixture::with_window();
    for _ in 1..AGGREGATE_CLIENTS {
        fixture.spawn(run_client);
    }
    fixture.batch(siblings(COUNT_BOUND));
    let Ack::Done = fixture.run_on(1, Step::MapWindow);
    for index in 1..AGGREGATE_CLIENTS {
        if index > 1 {
            let Ack::Done = fixture.run_on(index, Step::MapWindow);
        }
        let Ack::Done = fixture.run_on(index, Step::Batch(siblings(COUNT_BOUND)));
    }
    assert_eq!(
        indexed(&fixture),
        AGGREGATE_CLIENTS * COUNT_BOUND,
        "every client's popups filed, nobody disconnected"
    );
    assert_eq!(
        fixture.state.popup_count.in_flight(),
        (AGGREGATE_CLIENTS * COUNT_BOUND) as u32
    );
    // Every popup configured, on every connection: each `Map` waits for
    // every configure it names.
    fixture.map(0..COUNT_BOUND, None);
    for index in 1..AGGREGATE_CLIENTS {
        let Ack::Done = fixture.run_on(
            index,
            Step::Map {
                popups: (0..COUNT_BOUND).collect(),
                marked: None,
            },
        );
    }
    // One client's destroys forget exactly its own entries.
    let Ack::Done = fixture.run_on(1, Step::Batch((0..COUNT_BOUND).map(Op::Destroy).collect()));
    assert_eq!(indexed(&fixture), (AGGREGATE_CLIENTS - 1) * COUNT_BOUND);
    assert_eq!(
        fixture.state.popup_count.in_flight(),
        ((AGGREGATE_CLIENTS - 1) * COUNT_BOUND) as u32
    );
    still_serving(&mut fixture);
}
