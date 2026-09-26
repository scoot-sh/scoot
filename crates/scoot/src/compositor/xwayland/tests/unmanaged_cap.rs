//! The per-X-client unmanaged-window cap: one X client may hold at most
//! 128 live override-redirect windows; a menu past that is refused --
//! never drawn, never hit-tested, never sent frame callbacks -- while other
//! X clients' menus still draw, and unmapping (or dying) hands the bound
//! back.
//!
//! Live: needs the `Xwayland` binary on `PATH`, like every other suite
//! under `tests/`. The counter's own discipline -- admit at the bound,
//! refuse past it, release exactly once, drain on the server's death -- is
//! pinned hermetically in `toplevel_cap/tests.rs`; what this proves is the
//! wiring: real X clients mapping override-redirect windows through
//! `map_x11_unmanaged`, and only that.

use x11rb::protocol::xproto::Window as XWindow;

use super::live::{BLUE, BLUE_BGRA, Live, RED, live};
use super::x11::{Props, XClient, eventually};
use crate::compositor::xwayland::focus::x_client_key;

/// The most live override-redirect windows one X client may hold -- the
/// code's `MAX_X11_UNMANAGED_PER_CLIENT`, spelled out so this file does not
/// depend on it (the way `toplevel_cap/tests.rs` spells out the xdg cap).
const CAP: usize = 128;

/// An override-redirect menu.
fn menu(pixel: u32) -> Props {
    let mut props = Props::new(pixel);
    props.override_redirect = true;
    props
}

/// Whether X window `xid` is on the unmanaged draw list right now.
fn is_drawn(live: &Live, xid: XWindow) -> bool {
    live.fixture
        .state
        .x11_unmanaged
        .iter()
        .any(|known| known.window_id() == xid)
}

/// How many of `xids` are on the unmanaged draw list right now.
fn drawn_count(live: &Live, xids: &[XWindow]) -> usize {
    xids.iter().filter(|xid| is_drawn(live, **xid)).count()
}

/// The cap's live count for the X client that owns `xid`.
fn live_for(live: &Live, xid: XWindow) -> u32 {
    live.fixture
        .state
        .x11_unmanaged_cap
        .live_for(&x_client_key(xid))
}

/// A client mapping past the cap is refused -- its menu is never drawn,
/// hit-tested or counted -- while another X client beside it and a Wayland
/// client are still served; unmapping then hands the bound back.
#[test]
fn an_x_client_past_the_cap_is_refused_while_others_are_served() {
    let Some(mut live) = live("an_x_client_past_the_cap_is_refused_while_others_are_served") else {
        return;
    };

    // All 129 map requests up front, in order on one connection: which of
    // them the refusal lands on is then deterministic, and no per-window
    // wait stretches the suite.
    let props = menu(RED);
    let mut xids = Vec::with_capacity(CAP + 1);
    for _ in 0..CAP + 1 {
        xids.push(live.x.map(&props));
    }
    eventually(
        &mut live.fixture,
        "the first 128 menus reaching the draw list",
        |fixture| {
            fixture
                .state
                .x11_unmanaged_cap
                .live_for(&x_client_key(xids[0]))
                == CAP as u32
        },
    );
    live.drain();
    assert_eq!(
        drawn_count(&live, &xids),
        CAP,
        "more -- or fewer -- than the bound reached the draw list"
    );
    assert!(
        !is_drawn(&live, xids[CAP]),
        "the menu past the cap reached the draw list"
    );
    assert_eq!(
        live_for(&live, xids[0]),
        CAP as u32,
        "the refused map claimed a unit"
    );
    // The refused menu never enters the core either (it shares no path
    // with the managed cap, and must not leak into its count).
    assert!(
        live.fixture.state.windows.values().all(|window| window
            .x11_surface()
            .is_none_or(|x11| x11.window_id() != xids[CAP])),
        "the refused menu entered the core"
    );
    assert_eq!(
        live.fixture
            .state
            .x11_toplevel_cap
            .live_for(&x_client_key(xids[0])),
        0,
        "an unmanaged map touched the managed cap"
    );

    // Per X client: a second connection -- other client bits -- maps beside
    // a full one, and its menu still draws (newest on top, over the red).
    let other = XClient::connect(live.display);
    let other_xid = other.map(&menu(BLUE));
    eventually(
        &mut live.fixture,
        "the second X client's menu reaching the draw list",
        |fixture| {
            fixture
                .state
                .x11_unmanaged
                .iter()
                .any(|known| known.window_id() == other_xid && known.wl_surface().is_some())
        },
    );
    assert_eq!(live_for(&live, other_xid), 1);
    assert_eq!(
        live_for(&live, xids[0]),
        CAP as u32,
        "the second client's map touched the first client's count"
    );
    assert_eq!(
        live.pixel_at(10, 10),
        BLUE_BGRA,
        "the second client's menu is not drawn over the full client's"
    );

    // The Wayland path is untouched by the unmanaged count.
    let peer_id = live.map_peer("peer-beside-full-x");
    assert!(live.fixture.state.windows.contains_key(&peer_id));

    // Release on unmap: withdrawing one menu lets the full client's next
    // map in.
    live.x.unmap(xids[0]);
    eventually(
        &mut live.fixture,
        "the unmap freeing the bound",
        |fixture| {
            fixture
                .state
                .x11_unmanaged_cap
                .live_for(&x_client_key(xids[0]))
                == CAP as u32 - 1
        },
    );
    assert!(!is_drawn(&live, xids[0]));
    let freed = live.x.map(&props);
    eventually(
        &mut live.fixture,
        "the freed bound serving the client again",
        |fixture| {
            fixture
                .state
                .x11_unmanaged
                .iter()
                .any(|known| known.window_id() == freed)
        },
    );
    assert_eq!(live_for(&live, xids[1]), CAP as u32);
}

/// An X client that goes away takes its count with it: dropping the
/// connection drains what its menus held, and a new connection starts
/// from zero -- no stale charge against reincarnated window ids.
#[test]
fn a_dead_x_client_releases_its_count() {
    let Some(mut live) = live("a_dead_x_client_releases_its_count") else {
        return;
    };
    let props = menu(RED);
    let xids: Vec<XWindow> = (0..5).map(|_| live.x.map(&props)).collect();
    eventually(
        &mut live.fixture,
        "the menus reaching the draw list",
        |fixture| {
            fixture
                .state
                .x11_unmanaged_cap
                .live_for(&x_client_key(xids[0]))
                == xids.len() as u32
        },
    );
    drop(live.x);
    eventually(
        &mut live.fixture,
        "the dead client's count draining",
        |fixture| fixture.state.x11_unmanaged_cap.in_flight() == 0,
    );
    assert!(
        live.fixture.state.x11_unmanaged.is_empty(),
        "a dead client's menus stayed on the draw list"
    );

    // And the next client starts clean, whatever ids the server reuses.
    live.x = XClient::connect(live.display);
    let xid = live.x.map(&menu(BLUE));
    eventually(
        &mut live.fixture,
        "a new client's menu reaching the draw list",
        |fixture| {
            fixture
                .state
                .x11_unmanaged
                .iter()
                .any(|known| known.window_id() == xid)
        },
    );
    assert_eq!(live_for(&live, xid), 1);
}

/// The client key is the window id's client bits, for menus as for managed
/// windows: two menus one connection created share it, two connections'
/// do not -- the identity the cap charges to. Hermetic (no server): the
/// mask itself is pinned live by `the_resource_id_mask_is_the_servers`.
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
        "two menus of one connection key differently"
    );
    assert_ne!(
        x_client_key(0x0050_0001),
        x_client_key(0x0070_0001),
        "two connections key alike"
    );
}
