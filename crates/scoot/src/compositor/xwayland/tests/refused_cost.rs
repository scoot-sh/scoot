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
//!   two pools and two buffers, and no more. Both are made before scoot
//!   hears of the map: the window's backing pixmap has its pool and buffer
//!   from the moment it is realized (`xwl_shm_create_pixmap`), and the
//!   first commit swaps in a second. It never commits again, because a
//!   refused window is never sent a frame callback and XWayland posts a
//!   window's next frame only after its last one's callback.
//!
//! So the server's cost is two per mapped window whether scoot draws it or
//! not, and a runaway X client mapping override-redirect windows past its
//! cap still spends the server's budget. What these pin is the part that
//! holds: the cost is bounded per window and does not grow while the
//! windows sit refused, and other X clients stay served.

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

    const REFUSED: u32 = 112;
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
