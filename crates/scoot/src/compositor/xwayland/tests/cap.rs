//! The per-X-client toplevel cap: one X client may hold at most 128 live
//! managed windows; a window past that is refused its map, while other X
//! clients and Wayland clients are still served, and unmapping (or dying)
//! hands the bound back.
//!
//! Live: needs the `Xwayland` binary on `PATH`, like every other suite
//! under `tests/`. The counter's own discipline -- admit at the bound,
//! refuse past it, release exactly once -- is pinned hermetically in
//! `toplevel_cap/tests.rs`; what this proves is the wiring: real X clients
//! mapping through `map_x11_window`, and only that.

use x11rb::protocol::xproto::Window as XWindow;

use super::live::{BLUE, Live, RED, id_of_xid, live};
use super::peer::{Ack, Step};
use super::x11::{Props, XClient, eventually};
use crate::compositor::xwayland::focus::x_client_key;

/// The most live managed windows one X client may hold -- the code's
/// `MAX_X11_TOPLEVELS_PER_CLIENT`, spelled out so this file does not depend
/// on it (the way `toplevel_cap/tests.rs` spells out the xdg cap).
const CAP: usize = 128;

/// How many of `xids` the compositor manages right now.
fn managed_count(live: &Live, xids: &[XWindow]) -> usize {
    xids.iter()
        .filter(|xid| id_of_xid(&live.fixture.state, **xid).is_some())
        .count()
}

/// The cap's live count for the X client that owns `xid`.
fn live_for(live: &Live, xid: XWindow) -> u32 {
    live.fixture
        .state
        .x11_toplevel_cap
        .live_for(&x_client_key(xid))
}

/// A client mapping past the cap is refused -- its window never enters the
/// core, the taskbar, or the count -- while another X client beside it and
/// a Wayland client are still served; unmapping then hands the bound back.
#[test]
fn an_x_client_past_the_cap_is_refused_while_others_are_served() {
    let Some(mut live) = live("an_x_client_past_the_cap_is_refused_while_others_are_served") else {
        return;
    };
    assert!(matches!(live.fixture.run(Step::BindTaskbar), Ack::Done));

    // All 129 map requests up front, in order on one connection: which of
    // them the refusal lands on is then deterministic, and no per-window
    // wait stretches the suite.
    let mut xids = Vec::with_capacity(CAP + 1);
    for _ in 0..CAP + 1 {
        xids.push(live.x.map(&Props::new(RED)));
    }
    eventually(
        &mut live.fixture,
        "the first 128 X windows entering the layout",
        |fixture| {
            fixture
                .state
                .x11_toplevel_cap
                .live_for(&x_client_key(xids[0]))
                == CAP as u32
        },
    );
    live.drain();
    assert_eq!(
        managed_count(&live, &xids),
        CAP,
        "more -- or fewer -- than the bound entered the layout"
    );
    assert!(
        id_of_xid(&live.fixture.state, xids[CAP]).is_none(),
        "the window past the cap entered the layout"
    );
    assert_eq!(
        live_for(&live, xids[0]),
        CAP as u32,
        "the refused map claimed a unit"
    );
    // The refused window never reaches the taskbar either: poll, since 128
    // announcements do not necessarily arrive in one round trip.
    eventually(
        &mut live.fixture,
        "the taskbar settling on exactly the managed windows",
        |fixture| match fixture.run(Step::Toplevels) {
            Ack::Toplevels(list) => list.len() == CAP,
            other => panic!("expected a toplevel list, got {other:?}"),
        },
    );

    // Per X client: a second connection -- other client bits -- maps beside
    // a full one.
    let other = XClient::connect(live.display);
    let other_xid = other.map(&Props::new(BLUE));
    eventually(
        &mut live.fixture,
        "the second X client's window entering the layout",
        |fixture| id_of_xid(&fixture.state, other_xid).is_some(),
    );
    assert_eq!(live_for(&live, other_xid), 1);
    assert_eq!(
        live_for(&live, xids[0]),
        CAP as u32,
        "the second client's map touched the first client's count"
    );

    // The Wayland path is untouched by the X count.
    let peer_id = live.map_peer("peer-beside-full-x");
    assert!(live.fixture.state.windows.contains_key(&peer_id));

    // Release on unmap: withdrawing one managed window lets the full
    // client's next map in.
    live.x.unmap(xids[0]);
    eventually(
        &mut live.fixture,
        "the unmap freeing the bound",
        |fixture| {
            fixture
                .state
                .x11_toplevel_cap
                .live_for(&x_client_key(xids[0]))
                == CAP as u32 - 1
        },
    );
    assert!(id_of_xid(&live.fixture.state, xids[0]).is_none());
    let freed = live.x.map(&Props::new(RED));
    eventually(
        &mut live.fixture,
        "the freed bound serving the client again",
        |fixture| id_of_xid(&fixture.state, freed).is_some(),
    );
    assert_eq!(live_for(&live, xids[1]), CAP as u32);
}

/// An X client that goes away takes its count with it: dropping the
/// connection drains what its windows held, and a new connection starts
/// from zero -- no stale charge against reincarnated window ids.
#[test]
fn a_dead_x_client_releases_its_count() {
    let Some(mut live) = live("a_dead_x_client_releases_its_count") else {
        return;
    };
    let xids: Vec<XWindow> = (0..5).map(|_| live.x.map(&Props::new(RED))).collect();
    eventually(
        &mut live.fixture,
        "the X windows entering the layout",
        |fixture| {
            fixture
                .state
                .x11_toplevel_cap
                .live_for(&x_client_key(xids[0]))
                == xids.len() as u32
        },
    );
    drop(live.x);
    eventually(
        &mut live.fixture,
        "the dead client's count draining",
        |fixture| fixture.state.x11_toplevel_cap.in_flight() == 0,
    );
    assert!(
        live.fixture.state.windows.is_empty(),
        "a dead client's windows stayed in the layout"
    );

    // And the next client starts clean, whatever ids the server reuses.
    live.x = XClient::connect(live.display);
    let xid = live.x.map(&Props::new(BLUE));
    eventually(
        &mut live.fixture,
        "a new client's window entering the layout",
        |fixture| id_of_xid(&fixture.state, xid).is_some(),
    );
    assert_eq!(live_for(&live, xid), 1);
}

/// The client key is the window id's client bits: two windows one
/// connection created share it, two connections' do not -- the identity the
/// cap charges to. Hermetic (no server): the mask itself is pinned live by
/// `the_resource_id_mask_is_the_servers`.
#[test]
fn the_x_client_key_is_the_id_client_bits() {
    use crate::compositor::xwayland::focus::{X_CLIENT_RESOURCE_MASK, x_client_key};

    assert_eq!(
        x_client_key(0x0050_0001),
        0x0050_0001 & !X_CLIENT_RESOURCE_MASK
    );
    assert_eq!(
        x_client_key(0x0050_0001),
        x_client_key(0x0051_0002),
        "two windows of one connection key differently"
    );
    assert_ne!(
        x_client_key(0x0050_0001),
        x_client_key(0x0070_0001),
        "two connections key alike"
    );
}
