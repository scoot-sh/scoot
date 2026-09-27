//! The XWayland server's own Wayland-side budget: one Wayland connection
//! carrying every X client's windows is not held to one Wayland app's
//! bounds (`docs/backlog/resolved/xwayland-server-death-many-unmanaged-done.md`).
//!
//! Live: needs the `Xwayland` binary on `PATH`, like every other suite under
//! `tests/`. What the budget's arithmetic is -- and that past it the server
//! is still refused -- is pinned hermetically in `client_fds/tests.rs` and
//! `wl_buffers/tests.rs`; what this proves is the wiring on the real server:
//! the connection a live `Xwayland` makes is the one that gets the budget.

use x11rb::protocol::xproto::ConnectionExt as _;

use super::live::{RED, live};
use super::x11::{Props, XClient, eventually};
use crate::compositor::xwayland_budget::bound as xwayland_fd_bound;

/// The per-client bound every ordinary Wayland client gets, spelled out so
/// this file does not depend on it: the number the server used to be killed
/// at (`MAX_FDS_PER_CLIENT` and `MAX_BUFFERS_PER_CLIENT` both).
const ONE_APP: usize = 512;

/// How many X clients share the load, so no one of them reaches the
/// per-X-client unmanaged cap (128) and every window is really drawn.
const X_CLIENTS: usize = 3;

/// Override-redirect windows in all. Measured: each costs the server two
/// `wl_shm` pools (one fd each) and two `wl_buffer`s, so the 257th used to
/// be the one whose pool took the server past 512 and got it disconnected
/// with every X client's windows.
const WINDOWS: usize = 300;

/// Several X clients' menus adding up past one Wayland app's bounds leave
/// the XWayland server connected and every menu drawn -- where the server
/// used to be killed by the per-client fd bound at the 257th window, taking
/// every X client's windows with it.
#[test]
fn many_x_windows_past_one_apps_bounds_keep_the_server() {
    let Some(mut live) = live("many_x_windows_past_one_apps_bounds_keep_the_server") else {
        return;
    };
    // The budget scales with the fd table; on a table too small to hold it
    // the server's bound is the ordinary one, and this scene cannot pass by
    // design (see `xwayland_fd_bound`). Say so rather than fail.
    let bound = xwayland_fd_bound() as usize;
    if bound < 2 * WINDOWS + 16 {
        eprintln!(
            "many_x_windows_past_one_apps_bounds_keep_the_server: skipped -- this process's fd \
             table gives the XWayland server a bound of {bound}, too small for {WINDOWS} windows"
        );
        return;
    }
    let clients: Vec<XClient> = (1..X_CLIENTS)
        .map(|_| XClient::connect(live.display))
        .collect();
    let mut menu = Props::new(RED);
    menu.override_redirect = true;
    for i in 0..WINDOWS {
        match i % X_CLIENTS {
            0 => live.x.map(&menu),
            n => clients[n - 1].map(&menu),
        };
        // Pumped as it goes: a server whose Wayland socket fills blocks,
        // and with it the X round trip `map` makes.
        if i % 16 == 15 {
            live.fixture.settle();
        }
    }
    eventually(
        &mut live.fixture,
        "every menu reaching the draw list with a surface",
        |fixture| {
            fixture.state.xwm.is_none()
                || fixture
                    .state
                    .x11_unmanaged
                    .iter()
                    .filter(|window| window.wl_surface().is_some())
                    .count()
                    == WINDOWS
        },
    );
    live.drain();
    assert!(
        live.fixture.state.xwm.is_some(),
        "the XWayland server was disconnected with {WINDOWS} windows mapped"
    );
    assert_eq!(live.fixture.state.x11_unmanaged.len(), WINDOWS);

    // The premise, not just the outcome: the server really holds more than
    // one Wayland app may, so this scene is the one that used to kill it.
    let server = live
        .fixture
        .state
        .x11_unmanaged
        .iter()
        .find_map(|window| window.wl_surface())
        .and_then(|surface| smithay::reexports::wayland_server::Resource::client(&surface))
        .expect("the XWayland server's connection");
    let held = live.fixture.state.client_fds.held_by(&server.id()) as usize;
    assert!(
        held > ONE_APP,
        "the scene never took the server past one app's fd bound ({held} held)"
    );
    assert!(
        live.fixture.state.wl_buffers.buffers_in_flight() > ONE_APP,
        "the scene never took the server past one app's buffer bound"
    );

    // And every X client is still being served.
    for conn in std::iter::once(&live.x.conn).chain(clients.iter().map(|client| &client.conn)) {
        conn.get_input_focus()
            .expect("an X request")
            .reply()
            .expect("the X server still answers");
    }
}

/// The live server connection is the one whose outstanding acquire waits
/// are held to its own, scaled bound rather than one app's 64: 64 X windows
/// committing GPU frames at once used to be enough to disconnect the server
/// with every X window in the session. No GPU is needed to prove the
/// wiring -- the bound is read off the connection, not off a wait -- and
/// the arithmetic is `xwayland_budget/tests.rs`'s.
#[test]
fn the_server_gets_its_own_acquire_wait_bound() {
    use crate::compositor::drm_syncobj::{MAX_ACQUIRE_WAITS_PER_CLIENT, max_acquire_waits_for};
    use crate::compositor::xwayland_budget::acquire_waits_for;

    let Some(mut live) = live("the_server_gets_its_own_acquire_wait_bound") else {
        return;
    };
    let mut menu = Props::new(RED);
    menu.override_redirect = true;
    live.x.map(&menu);
    eventually(&mut live.fixture, "the menu's surface", |fixture| {
        fixture
            .state
            .x11_unmanaged
            .iter()
            .any(|window| window.wl_surface().is_some())
    });
    let server = live
        .fixture
        .state
        .x11_unmanaged
        .iter()
        .find_map(|window| window.wl_surface())
        .and_then(|surface| smithay::reexports::wayland_server::Resource::client(&surface))
        .expect("the XWayland server's connection");
    let bound = xwayland_fd_bound();
    assert_eq!(max_acquire_waits_for(&server), acquire_waits_for(bound));
    if bound > ONE_APP as u32 {
        assert!(
            max_acquire_waits_for(&server) > MAX_ACQUIRE_WAITS_PER_CLIENT,
            "the server is held to one app's acquire waits on a {bound}-fd budget"
        );
    } else {
        eprintln!(
            "the_server_gets_its_own_acquire_wait_bound: this process's fd table gives the \
             server one app's budget ({bound}), so its bound is one app's too"
        );
    }
}
