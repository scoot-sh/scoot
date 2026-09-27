//! The XWayland server's budget, hermetically: its arithmetic, that it
//! leaves the 1024-fd table's margins exactly as they were, and that the
//! ledger and the buffer count enforce it -- the server is held to a bigger
//! bound, not to none. That a live `Xwayland` connection is the one that
//! gets it is `xwayland/tests/server_budget.rs`.

use std::sync::Arc;

use smithay::reexports::wayland_server::backend::ClientData;
use smithay::reexports::wayland_server::{Client, Display};

use super::{MAX, MIN, bound_for, is_server};
use crate::compositor::client_fds::{
    Check, ClientFds, Kind, LIMITS, MAX_FDS_PER_CLIENT, Refusal, limits_for, xwayland_limits,
};
use crate::compositor::fd_pressure::RESERVE_FDS;
use crate::compositor::nofile::RAISED_SOFT_CAP;
use crate::compositor::wl_buffers::{MAX_BUFFERS_PER_CLIENT, WlBuffers, max_buffers_for};

/// An ordinary client: scoot's `ClientState` is not needed, only data that
/// is not XWayland's.
fn ordinary_client() -> (Display<()>, Client) {
    struct Data;
    impl ClientData for Data {}
    let display: Display<()> = Display::new().expect("a display");
    let (server, _peer) = std::os::unix::net::UnixStream::pair().expect("a socket pair");
    let client = display
        .handle()
        .insert_client(server, Arc::new(Data))
        .expect("a client");
    (display, client)
}

#[test]
fn the_budget_is_a_sixteenth_of_the_table_clamped() {
    assert_eq!(MIN, MAX_FDS_PER_CLIENT);
    assert_eq!(bound_for(0), MIN);
    assert_eq!(bound_for(1024), MIN);
    assert_eq!(bound_for(8192), MIN);
    assert_eq!(bound_for(8192 + 16), MIN + 1);
    assert_eq!(bound_for(20_000), 1250);
    assert_eq!(bound_for(RAISED_SOFT_CAP), MAX);
    assert_eq!(bound_for(1 << 20), MAX);
    // `RLIM_INFINITY` reads as `u64::MAX`: clamped, never truncated.
    assert_eq!(bound_for(u64::MAX), MAX);
}

/// Where the hard limit keeps the table at 1024, the server's bounds are
/// the ordinary ones, so every margin `fd_pressure.rs` and `client_fds.rs`
/// derive for that table (one connection at every bound stays under the
/// pressure line) holds for it unchanged.
#[test]
fn a_1024_fd_table_gives_the_server_one_apps_bounds() {
    assert_eq!(bound_for(1024), MAX_FDS_PER_CLIENT);
    assert_eq!(bound_for(1024), MAX_BUFFERS_PER_CLIENT);
    assert_eq!(xwayland_limits(bound_for(1024)).total, LIMITS.total);
}

/// On any table big enough to raise it, the server at its whole budget is a
/// sixteenth of the table at most: nowhere near the pressure line, which
/// sits `RESERVE_FDS` below the table's end.
#[test]
fn the_server_alone_cannot_pressure_the_table() {
    for soft in [1024, 4096, 8192, 20_000, 32_768, RAISED_SOFT_CAP] {
        let bound = u64::from(bound_for(soft));
        assert!(
            bound <= (soft / 16).max(u64::from(MIN)),
            "{soft}-fd table: {bound}"
        );
        if bound > u64::from(MIN) {
            assert!(
                bound * 8 < soft - RESERVE_FDS,
                "{soft}-fd table: the server's {bound} is not small next to the pressure line"
            );
        }
    }
}

/// The ledger holds the server to its budget: past one app's 512 fds and
/// 128 timelines it is admitted, and at the budget it is refused -- with
/// the refusal naming the budget, not 512.
#[test]
fn the_ledger_admits_the_server_past_one_app_and_refuses_it_at_its_budget() {
    let (_display, client) = ordinary_client();
    let id = client.id();
    let budget = bound_for(RAISED_SOFT_CAP);
    let limits = xwayland_limits(budget);
    let mut ledger = ClientFds::default();
    // Timelines first, past the ordinary 128, then pools up to the budget.
    for fd in 0..200 {
        assert_eq!(
            ledger.admit(&id, Kind::Timeline, 1, limits, |_, _| true, || false),
            Ok(()),
            "timeline {fd}"
        );
        ledger.record(&id, fd, Kind::Timeline, Check::Open, 1);
    }
    for fd in 200..budget as i32 {
        assert_eq!(
            ledger.admit(&id, Kind::Pool, 1, limits, |_, _| true, || false),
            Ok(()),
            "pool {fd}"
        );
        ledger.record(&id, fd, Kind::Pool, Check::Open, 1);
    }
    assert_eq!(ledger.held_by(&id), budget);
    assert_eq!(
        ledger.admit(&id, Kind::Pool, 1, limits, |_, _| true, || false),
        Err(Refusal::Total {
            held: budget,
            max: budget
        })
    );
    // The same ledger under the ordinary bounds would have refused long
    // before.
    assert!(matches!(
        ledger.admit(&id, Kind::Pool, 1, LIMITS, |_, _| true, || false),
        Err(Refusal::Total { max: 512, .. })
    ));
    let message = Refusal::Total {
        held: budget,
        max: budget,
    }
    .message("wl_shm pool refused");
    assert!(
        message.ends_with(&format!("the maximum is {budget}")),
        "{message}"
    );
}

/// The live-buffer count holds the server to its budget the same way.
#[test]
fn the_buffer_count_admits_the_server_past_one_app_and_refuses_it_at_its_budget() {
    let (_display, client) = ordinary_client();
    let budget = bound_for(RAISED_SOFT_CAP);
    let mut buffers = WlBuffers::default();
    for n in 0..budget {
        assert!(!buffers.claim(client.id(), budget), "buffer {n}");
    }
    assert!(buffers.claim(client.id(), budget), "past the budget");
    assert_eq!(buffers.buffers_in_flight(), budget as usize);
}

/// Every connection that is not the server keeps one app's bounds: the
/// budget is keyed on Smithay's `XWaylandClientData`, which only
/// `XWayland::spawn` inserts.
#[test]
fn an_ordinary_client_keeps_one_apps_bounds() {
    let (_display, client) = ordinary_client();
    assert!(!is_server(&client));
    assert_eq!(limits_for(&client), LIMITS);
    assert_eq!(max_buffers_for(&client), MAX_BUFFERS_PER_CLIENT);
}
