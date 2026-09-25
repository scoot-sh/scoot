//! Tests for the global fd ceiling's observation and predicate.
//!
//! The decision core is pure ([`Table::pressured`] over a hand-built
//! [`Table`](super::Table)), so every boundary is pinned exactly; the one
//! live test asserts the observer agrees with the kernel about this very
//! process. Enforcement-site wiring (what a shed/refusal does on the wire)
//! is pinned at each site, not here.

use super::*;

mod backend_queue;
mod backend_queue_client;

/// The most fds one read from a client's socket can add to that queue: 30.
/// wayland-backend sizes its receive buffer for 28 (its `MAX_FDS_OUT`) with
/// rustix's `cmsg_space!`, which pads it by the 8 bytes of `cmsghdr`
/// alignment, so depending on where the buffer lands the kernel has room
/// for 28 to 30, and closes the rest of a larger message. So a connection
/// can hold its cap (`backend_queued_fds`) + 30 unclaimed for a moment,
/// inside the read that takes it past the cap. Pinned by `backend_queue.rs`.
const BACKEND_READ_FDS: u64 = 30;

/// A table at the dev-VM size, `free` fds standing free.
fn table_with_free(free: u64) -> Table {
    Table {
        used: 1024 - free,
        soft: 1024,
    }
}

#[test]
fn plenty_free_is_calm() {
    assert!(!table_with_free(1024).pressured());
    assert!(!table_with_free(129).pressured());
}

#[test]
fn the_reserve_boundary_trips_exactly() {
    // One free fd below the reserve sheds; exactly the reserve does not.
    // The boundary is `<`, not `<=`: `RESERVE_FDS` free means the reserve
    // is intact.
    assert!(!table_with_free(RESERVE_FDS).pressured());
    assert!(table_with_free(RESERVE_FDS - 1).pressured());
}

#[test]
fn a_full_table_is_pressured() {
    assert!(table_with_free(0).pressured());
}

#[test]
fn used_past_soft_reads_as_zero_free_not_a_wrap() {
    // A limit shrunk under current use (or a racing count) must read as
    // pressured, never as `u64::MAX` free via wrap.
    let table = Table {
        used: 2000,
        soft: 1024,
    };
    assert_eq!(table.free(), 0);
    assert!(table.pressured());
}

#[test]
fn a_small_table_is_calm_whatever_it_holds() {
    // Mirrors `table()`'s `None` for the same table: below `MIN_TABLE_FDS`
    // there is no guard, so the predicate must not fire either, or a
    // hand-built reading and the observer would disagree about identical
    // input.
    for soft in [0, 1, 64, MIN_TABLE_FDS - 1] {
        assert!(
            !Table { used: soft, soft }.pressured(),
            "soft {soft} must read calm"
        );
        assert!(
            !Table {
                used: soft + 1000,
                soft
            }
            .pressured(),
            "soft {soft} overfull must still read calm"
        );
    }
    assert!(
        Table {
            used: MIN_TABLE_FDS,
            soft: MIN_TABLE_FDS
        }
        .pressured()
    );
}

#[test]
fn free_is_soft_minus_used() {
    let table = Table {
        used: 896,
        soft: 1024,
    };
    assert_eq!(table.free(), 128);
    assert!(!table.pressured(), "128 free is the intact reserve");
    let table = Table {
        used: 897,
        soft: 1024,
    };
    assert_eq!(table.free(), 127);
    assert!(table.pressured(), "127 free spends the reserve");
}

#[test]
fn the_live_observer_agrees_with_the_kernel() {
    let table = table().expect("the observer works on this machine");
    assert!(
        table.soft >= MIN_TABLE_FDS,
        "a guarded table is what the observer returns: {table:?}"
    );
    // The process holds at least this test's own stack of fds; and the
    // count cannot exceed the limit it is measured against by more than
    // racing noise -- `used <= soft` up to the counting dir fd itself.
    assert!(table.used >= 1, "an empty table reading is a broken gauge");
    assert!(
        table.used <= table.soft + 8,
        "the count overshoots the limit it was read against: {table:?}"
    );
    // Cross-check one independent primitive: the soft limit the observer
    // reports is the one `prlimit` reports for this process.
    let soft = soft_limit().expect("getrlimit works on this machine");
    assert_eq!(table.soft, soft);
}

/// The claim the module doc's numbers rest on: one connection at every
/// per-client bound at once, on the tier with the most (the `--tty` GPU
/// scanout tier, with explicit sync offered), plus that tier's measured idle
/// baseline, stays below the reserve line, on both tables scoot runs with:
/// the raised one ([`RAISED_SOFT_CAP`](crate::compositor::nofile::RAISED_SOFT_CAP))
/// and a 1024-fd table where the hard limit allows no raise. That includes
/// the fds wayland-backend holds for it below scoot: its received-fd queue at
/// the most the fork allows on that table ([`backend_queued_fds`]), plus the
/// one read [`BACKEND_READ_FDS`] that can sit on top for a moment.
///
/// The fd figure is not read off a constant: it is what the fd ledger's own
/// admission rule lets one client reach, driven here with every kind and
/// weight a client can bring (pools, timelines, and planes weighing their
/// renderer copies too, up to the probe's cap of four per backend) until it
/// refuses. Review of PR #239 found the first version of this test only
/// added constants, while the code let imports charge copies after the bound
/// was checked and reach 540. On `main` before the ledger the same sum was
/// 512 buffers + 128 pools + 32 pending planes + 128 timelines + 64 waits +
/// 1 = 865, and 908 with the baseline: past the line.
#[test]
fn one_connection_at_every_bound_stays_below_the_reserve() {
    use crate::compositor::client_fds::{Check, ClientFds, Kind, LIMITS, MAX_FDS_PER_CLIENT};
    use crate::compositor::drm_syncobj::MAX_ACQUIRE_WAITS_PER_CLIENT;
    use smithay::reexports::wayland_server::Display;
    /// Idle `--tty --renderer gles` on the dev VM, 2026-09-24.
    const GPU_TIER_IDLE_BASELINE: u64 = 43;
    const SOCKET: u64 = 1;

    struct Data;
    impl smithay::reexports::wayland_server::backend::ClientData for Data {}
    let display: Display<()> = Display::new().expect("a display");
    let (server, _client) = std::os::unix::net::UnixStream::pair().expect("a socket pair");
    let client = display
        .handle()
        .insert_client(server, std::sync::Arc::new(Data))
        .expect("a client")
        .id();

    // Every arrival stays open (the worst case), with no pressure.
    let arrivals = [
        (Kind::Pool, 1u8),
        (Kind::Plane, 2),
        (Kind::Timeline, 1),
        (Kind::Plane, 5),
        (Kind::Plane, 3),
    ];
    let mut ledger = ClientFds::default();
    let mut reached = 0;
    for (n, fd) in (0..100_000).enumerate() {
        let (kind, weight) = arrivals[n % arrivals.len()];
        let admitted = ledger.admit(&client, kind, weight, LIMITS, |_, _| true, || false);
        if admitted.is_ok() {
            ledger.record(&client, fd, kind, Check::Open, weight);
        } else if kind == Kind::Plane && weight == 2 {
            // Refused at the smallest plane weight: nothing heavier fits.
            break;
        }
        reached = reached.max(ledger.held_by(&client));
    }
    assert!(
        reached <= MAX_FDS_PER_CLIENT,
        "the ledger let one client reach {reached}"
    );
    let one_connection = u64::from(reached) + u64::from(MAX_ACQUIRE_WAITS_PER_CLIENT) + SOCKET;
    // The module doc's figures. On 1024: 577 counted, 620 with the baseline,
    // 748 with a full backend queue at rest and 778 inside the read past it.
    // On 65536: 1601 at rest, 1631 inside the read, 1674 with the baseline.
    for soft in [1024, crate::compositor::nofile::RAISED_SOFT_CAP] {
        let line = soft - RESERVE_FDS;
        let with_queue = one_connection + backend_queued_fds(soft) + BACKEND_READ_FDS;
        assert!(
            with_queue + GPU_TIER_IDLE_BASELINE < line,
            "on a {soft}-fd table, {with_queue} + {GPU_TIER_IDLE_BASELINE} (the backend's \
             queue included) is past the {line} line"
        );
    }
}
