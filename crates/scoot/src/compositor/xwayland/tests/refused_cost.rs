//! What a window scoot refuses under the per-X-client caps
//! (`toplevel_cap.rs`) costs the XWayland server's Wayland connection --
//! the budget `xwayland_budget.rs` holds it to.
//!
//! Live: needs the `Xwayland` binary on `PATH`, like every other suite
//! under `tests/`. Measured with these same scenes (see
//! `docs/backlog/protocols/xwayland-refused-windows-still-commit.md`):
//!
//! - A refused **managed** window costs nothing. Its map request is never
//!   granted, so its frame window is never mapped, XWayland never realizes
//!   it, and no `wl_surface`, pool or buffer is made for it.
//! - A refused **override-redirect** window costs what a drawn one does,
//!   and no more -- up to the kill tolerance past the cap. Both are
//!   made before scoot hears of the map: the window's backing pixmap has
//!   its pool and buffer from the moment it is realized
//!   (`xwl_shm_create_pixmap`), and the first commit hands it over (a
//!   second follows on the next frame, which never comes for a refused
//!   window: it is never sent a frame callback, and XWayland posts a
//!   window's next frame only after its last one's callback -- measured
//!   here, one mapped menu holds one buffer and one fd once settled).
//! - Past the tolerance (64 refused menus past the 128 cap) the runaway
//!   client's X connection is killed (`XKillClient`), so one X client
//!   mapping menus far past its cap cannot spend the server to its budget
//!   on its own. Closing refused windows one by one was measured and does
//!   not keep up (a paced 5000-menu storm still disconnects with thousands
//!   closed -- the frees lag the maps), so the source has to stop. Below
//!   the tolerance a refused menu is merely refused, and costs what a
//!   drawn one does.
//!
//! So below the tolerance the server's cost is bounded per mapped window
//! whether scoot draws it or not, and what the pins hold is the part that
//! keeps: the cost does not grow while refused windows sit, other X
//! clients stay served -- and far past the cap the runaway is killed with
//! the server still connected and other clients still served.

use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ClientId;
use x11rb::protocol::xproto::ConnectionExt as _;

use super::live::{Live, RED, id_of_xid, live};
use super::x11::{Props, XClient, eventually};

/// The per-X-client caps, managed and override-redirect alike -- the
/// code's `MAX_X11_TOPLEVELS_PER_CLIENT` and
/// `MAX_X11_UNMANAGED_PER_CLIENT`, spelled out so this file does not
/// depend on them.
const CAP: usize = 128;

/// What one mapped X window costs the server, measured: two `wl_shm` pools
/// (one fd each) and two `wl_buffer`s.
const PER_WINDOW: u32 = 2;

/// The XWayland server's connection, found through any X window's surface.
fn server(live: &Live) -> ClientId {
    let state = &live.fixture.state;
    state
        .x11_unmanaged
        .iter()
        .find_map(|window| window.wl_surface())
        .or_else(|| {
            state
                .windows
                .values()
                .find_map(|window| window.x11_surface().and_then(|x11| x11.wl_surface()))
        })
        .and_then(|surface| surface.client())
        .map(|client| client.id())
        .expect("the XWayland server's connection")
}

/// The server's live buffers and ledger fds.
fn cost(live: &Live, server: &ClientId) -> (u32, u32) {
    let state = &live.fixture.state;
    (
        state.wl_buffers.live_for(server),
        state.client_fds.held_by(server),
    )
}

/// Every X client is still being answered.
fn all_served(clients: &[&XClient]) {
    for client in clients {
        client
            .conn
            .get_input_focus()
            .expect("an X request")
            .reply()
            .expect("the X server still answers");
    }
}

/// One X client mapping 72 managed windows past its cap costs the server
/// only what its 128 managed windows do; another X client is still served.
#[test]
fn a_refused_managed_window_costs_the_server_nothing() {
    let Some(mut live) = live("a_refused_managed_window_costs_the_server_nothing") else {
        return;
    };
    let props = Props::new(RED);
    let mut xids = Vec::with_capacity(CAP + 72);
    for i in 0..CAP + 72 {
        xids.push(live.x.map(&props));
        if i % 16 == 15 {
            live.fixture.settle();
        }
    }
    eventually(&mut live.fixture, "128 managed windows", |fixture| {
        xids.iter()
            .filter(|xid| id_of_xid(&fixture.state, **xid).is_some())
            .count()
            == CAP
    });
    live.drain();
    let server = server(&live);
    let (buffers, fds) = cost(&live, &server);
    // Measured 256 buffers and 258 fds (a resize or two costs a pool more).
    let drawn = PER_WINDOW * CAP as u32;
    assert!(
        buffers <= drawn + 8 && fds <= drawn + 8,
        "128 managed windows and 72 refused cost the server {buffers} buffers and {fds} fds, \
         more than the {drawn} the managed ones do"
    );

    let other = XClient::connect(live.display);
    let xid = other.map(&props);
    eventually(
        &mut live.fixture,
        "the other X client's window",
        |fixture| id_of_xid(&fixture.state, xid).is_some(),
    );
    all_served(&[&live.x, &other]);
}

/// One X client mapping override-redirect windows past its cap: each
/// refused one costs the server no more than a drawn one, that cost does
/// not grow while it sits refused, and another X client's menu still draws.
#[test]
fn a_refused_menu_costs_the_server_no_more_than_a_drawn_one() {
    let Some(mut live) = live("a_refused_menu_costs_the_server_no_more_than_a_drawn_one") else {
        return;
    };
    let mut menu = Props::new(RED);
    menu.override_redirect = true;
    for i in 0..CAP {
        live.x.map(&menu);
        if i % 16 == 15 {
            live.fixture.settle();
        }
    }
    eventually(&mut live.fixture, "128 menus drawn", |fixture| {
        fixture.state.x11_unmanaged.len() == CAP
            && fixture
                .state
                .x11_unmanaged
                .iter()
                .all(|window| window.wl_surface().is_some())
    });
    live.drain();
    let server = server(&live);
    let (at_cap, _) = cost(&live, &server);

    // Under the kill tolerance (64 refused past the cap): all tolerated,
    // none killed -- the kill storm is its own test below.
    const REFUSED: u32 = 64;
    for i in 0..REFUSED {
        live.x.map(&menu);
        if i % 16 == 15 {
            live.fixture.settle();
        }
    }
    live.drain();
    assert_eq!(live.fixture.state.x11_unmanaged.len(), CAP, "none admitted");
    let (refused, refused_fds) = cost(&live, &server);
    assert!(
        refused <= at_cap + PER_WINDOW * REFUSED,
        "{REFUSED} refused menus took the server from {at_cap} to {refused} buffers"
    );
    // Sitting refused, they commit nothing more: no frame callback ever
    // comes for them.
    for _ in 0..5 {
        live.drain();
    }
    assert_eq!(cost(&live, &server), (refused, refused_fds));

    let other = XClient::connect(live.display);
    let xid = other.map(&menu);
    eventually(&mut live.fixture, "the other X client's menu", |fixture| {
        fixture
            .state
            .x11_unmanaged
            .iter()
            .any(|window| window.window_id() == xid)
    });
    all_served(&[&live.x, &other]);
}

/// One X client mapping menus far past its cap -- past what the server's
/// budget holds -- does not take the server: past the kill tolerance its X
/// connection is killed (`XKillClient`), so its windows go with it and it
/// maps no more, while the server and other X clients stay served.
///
/// Live: needs the `Xwayland` binary on `PATH`. Fails without the kill
/// (the runaway maps until the server is disconnected past its budget,
/// and every X connection breaks), passes with it (the runaway is killed
/// near 192 maps, the server stays connected, and another X client is
/// still served).
#[test]
fn a_runaway_menu_storm_does_not_take_the_server() {
    let Some(mut live) = live("a_runaway_menu_storm_does_not_take_the_server") else {
        return;
    };
    let mut menu = Props::new(RED);
    menu.override_redirect = true;
    // Past the largest server budget (4096): without the kill the runaway
    // maps until the server is disconnected on every table; with it the
    // runaway is killed near 128 + 64 maps on every table. Measured, one
    // mapped override-redirect window costs the server 1 buffer and 1 fd
    // here, so 5000 maps exceed even the largest budget without the kill.
    const STORM: usize = 5000;
    let mut mapped = 0;
    for i in 0..STORM {
        if live.x.try_map(&menu).is_none() {
            break;
        }
        mapped += 1;
        if i % 16 == 15 {
            live.fixture.settle();
        }
    }
    // With the kill the runaway is dead long before the storm ends (near
    // 193 maps: 128 drawn + 64 tolerated + the one that trips the kill);
    // without it the runaway maps until the server dies (512+ maps even
    // on the smallest table). Either way its connection is broken by now
    // -- what distinguishes the two is whether the server survived for
    // others.
    assert!(
        mapped < 300,
        "the runaway mapped {mapped} menus before its connection broke: the kill did not fire"
    );
    live.drain();
    // The server's own Wayland client (not via the draw list: the kill
    // destroyed every drawn window with the runaway, so no surface is
    // left to trace it through).
    let server = live
        .fixture
        .state
        .xwayland_client
        .as_ref()
        .map(|client| client.id())
        .expect("the XWayland server's connection");
    let (buffers, fds) = cost(&live, &server);
    let bound = crate::compositor::xwayland_budget::bound();
    assert!(
        buffers <= bound && fds <= bound,
        "the storm holds the server at {buffers} buffers and {fds} fds, past its {bound} budget"
    );

    let other = XClient::connect(live.display);
    let mut other_menu = Props::new(RED);
    other_menu.override_redirect = true;
    let xid = other.map(&other_menu);
    eventually(&mut live.fixture, "the other X client's menu", |fixture| {
        fixture
            .state
            .x11_unmanaged
            .iter()
            .any(|window| window.window_id() == xid)
    });
    all_served(&[&other]);
}
