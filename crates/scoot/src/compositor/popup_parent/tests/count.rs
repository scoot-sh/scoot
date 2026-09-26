//! How many popups one client may hold: [`COUNT_BOUND`] live popups are
//! served, one more disconnects it, and the count is per client.
//!
//! Every test drives a real `wayland-client` against a real headless
//! compositor the way a hostile client would: thousands of popups side by
//! side in one flush (all within the depth cap, so the depth bound cannot
//! help). Each asserts on what the client was told -- the protocol error
//! that ended it -- and on the count beside the wire: the client past the
//! cap is disconnected with `wl_display.no_memory`, everything it held is
//! released, and other clients are still served.
//!
//! The tests drive the spelled-out [`COUNT_BOUND`] rather than the code's
//! cap, which is how they were watched failing first; the only import of
//! `popup_count::MAX_POPUPS_PER_CLIENT` is the drift assert, added after
//! the watch, checking the two still match.

use super::*;
use crate::compositor::popup_count::MAX_POPUPS_PER_CLIENT;

/// The most live popups one client may hold --
/// `popup_count::MAX_POPUPS_PER_CLIENT`, spelled out so this file does not
/// depend on it (see the module doc).
const COUNT_BOUND: usize = 128;

/// `wl_display.error.no_memory`.
const NO_MEMORY: u32 = 2;

/// A batch of `count` side-by-side popups of the window, all sent in one
/// flush.
fn siblings(count: usize) -> Vec<Op> {
    std::iter::repeat_n(Op::Popup(Parent::Window(0)), count).collect()
}

fn live(fixture: &Fixture, index: usize) -> u32 {
    fixture
        .state
        .popup_count
        .live_for(&fixture.client(index).id())
}

/// A popup past the cap disconnects its client with `wl_display.no_memory`,
/// and everything it held -- its count above all -- goes with it. The
/// teardown path is the one that matters here: the backend destroys the
/// dead client's objects with no client left to charge anything to, so the
/// release reads the client back out of each popup's own record.
#[test]
fn a_popup_past_the_cap_disconnects_with_no_memory() {
    assert_eq!(
        MAX_POPUPS_PER_CLIENT, COUNT_BOUND as u32,
        "the test's spelled-out cap drifted from the code's"
    );
    let mut fixture = Fixture::with_window();
    fixture.batch(siblings(COUNT_BOUND));
    assert_eq!(live(&fixture, 0), COUNT_BOUND as u32);
    assert_eq!(fixture.state.popup_count.in_flight(), COUNT_BOUND as u32);
    let error = fixture.refused(siblings(1));
    assert!(
        error.contains("wl_display")
            && error.contains(&format!("Protocol error {NO_MEMORY}"))
            && error.contains(&format!("at most {COUNT_BOUND}")),
        "{error}"
    );
    assert_eq!(
        fixture.state.popup_count.in_flight(),
        0,
        "the killed client's count drained"
    );
}

/// Exactly the bound is allowed: the 128th popup is configured, drawn, and
/// served like any other.
#[test]
fn exactly_the_bound_is_allowed() {
    let mut fixture = Fixture::with_window();
    fixture.batch(siblings(COUNT_BOUND));
    assert_eq!(live(&fixture, 0), COUNT_BOUND as u32);
    fixture.map(0..COUNT_BOUND, Some(COUNT_BOUND - 1));
    fixture.render();
    still_serving(&mut fixture);
}

/// Per client: one client at its bound does not stop another from opening,
/// and one client's destroys never touch another's count.
#[test]
fn the_bound_is_per_client() {
    let mut fixture = Fixture::with_window();
    fixture.spawn(run_client);
    fixture.batch(siblings(COUNT_BOUND));
    let Ack::Done = fixture.run_on(1, Step::MapWindow);
    let Ack::Done = fixture.run_on(1, Step::Batch(siblings(5)));
    assert_eq!(live(&fixture, 0), COUNT_BOUND as u32);
    assert_eq!(live(&fixture, 1), 5, "the second client opened beside it");
    assert_eq!(
        fixture.state.popup_count.in_flight(),
        COUNT_BOUND as u32 + 5
    );
    let Ack::Done = fixture.run_on(1, Step::Batch((0..5).map(Op::Destroy).collect()));
    assert_eq!(live(&fixture, 1), 0);
    assert_eq!(
        live(&fixture, 0),
        COUNT_BOUND as u32,
        "the other's destroys left this count alone"
    );
}

/// Destroying popups hands the bound back: a client that filled it,
/// destroyed everything, and filled it again is served throughout.
#[test]
fn destroyed_popups_release_the_bound() {
    let mut fixture = Fixture::with_window();
    fixture.batch(siblings(COUNT_BOUND));
    fixture.batch((0..COUNT_BOUND).map(Op::Destroy).collect());
    assert_eq!(live(&fixture, 0), 0);
    assert_eq!(fixture.state.popup_count.in_flight(), 0);
    fixture.batch(siblings(COUNT_BOUND));
    assert_eq!(live(&fixture, 0), COUNT_BOUND as u32);
    still_serving(&mut fixture);
}

/// Re-making a popup over its old surface nets nothing: the old popup's
/// destroy releases the claim before the new one's creation takes it.
#[test]
fn reincarnating_a_popup_keeps_the_count() {
    let mut fixture = Fixture::with_window();
    fixture.batch(siblings(COUNT_BOUND));
    fixture.batch(vec![Op::Reincarnate {
        popup: 0,
        parent: Parent::Window(0),
    }]);
    assert_eq!(
        live(&fixture, 0),
        COUNT_BOUND as u32,
        "the destroy and the re-creation cancelled out"
    );
    fixture.map(0..COUNT_BOUND, None);
    fixture.render();
    still_serving(&mut fixture);
}
